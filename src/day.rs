use std::fmt;

use encoding_rs::SHIFT_JIS;

const HEADER_LEN: usize = 8;
const RECORD_HEADER_LEN: usize = 5;
const XOR_KEY: u8 = 0x12;
const SIGNATURE: &[u8; 3] = b"N20";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DayRecord {
    pub day: u8,
    pub flags: u16,
    pub text: String,
    pub original_text: Option<String>,
    pub original_text_bytes: Option<Vec<u8>>,
    pub decode_had_errors: bool,
}

impl DayRecord {
    pub fn new(day: u8, flags: u16, text: String) -> Self {
        Self {
            day,
            flags,
            text,
            original_text: None,
            original_text_bytes: None,
            decode_had_errors: false,
        }
    }

    pub fn weather_code(&self) -> u8 {
        ((self.flags >> 11) & 0x1f) as u8
    }

    pub fn mood_code(&self) -> u8 {
        ((self.flags >> 7) & 0x0f) as u8
    }

    pub fn is_anniversary(&self) -> bool {
        self.flags & 0x0040 != 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DayFile {
    pub records: Vec<DayRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DayFormatError {
    TooShort,
    InvalidSignature,
    PayloadLengthMismatch { declared: usize, actual: usize },
    TrailingBodyBytes { count: usize },
    TruncatedRecordHeader { index: usize },
    InvalidDay { index: usize, day: u8 },
    EmptyTextRecord { index: usize },
    TruncatedRecordText { index: usize },
    MissingTextTerminator { index: usize },
    TextTooLong { index: usize },
    TooManyRecords,
    PayloadTooLarge,
    InvalidWeatherCode,
    InvalidMoodCode,
    EmbeddedNul,
    TextNotRepresentableInShiftJis,
}

impl fmt::Display for DayFormatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort => write!(formatter, "file is shorter than its 8-byte header"),
            Self::InvalidSignature => write!(formatter, "signature is not N20"),
            Self::PayloadLengthMismatch { declared, actual } => write!(
                formatter,
                "declared body length {declared} does not match actual length {actual}"
            ),
            Self::TrailingBodyBytes { count } => {
                write!(
                    formatter,
                    "{count} unparsed bytes remain after the final record"
                )
            }
            Self::TruncatedRecordHeader { index } => {
                write!(formatter, "record {index} has a truncated header")
            }
            Self::InvalidDay { index, day } => {
                write!(formatter, "record {index} has invalid day {day}")
            }
            Self::EmptyTextRecord { index } => {
                write!(formatter, "record {index} has a zero-length text field")
            }
            Self::TruncatedRecordText { index } => {
                write!(formatter, "record {index} has truncated text bytes")
            }
            Self::MissingTextTerminator { index } => {
                write!(formatter, "record {index} is missing its trailing NUL")
            }
            Self::TextTooLong { index } => {
                write!(formatter, "record {index} text exceeds 65535 bytes")
            }
            Self::TooManyRecords => write!(formatter, "record count exceeds 255"),
            Self::PayloadTooLarge => write!(formatter, "body exceeds the 32-bit file limit"),
            Self::InvalidWeatherCode => write!(formatter, "weather code exceeds 5 bits"),
            Self::InvalidMoodCode => write!(formatter, "mood code exceeds 4 bits"),
            Self::EmbeddedNul => write!(formatter, "edited text cannot contain NUL"),
            Self::TextNotRepresentableInShiftJis => {
                write!(
                    formatter,
                    "edited text contains characters not representable in Shift-JIS"
                )
            }
        }
    }
}

impl std::error::Error for DayFormatError {}

pub fn flags_with_metadata(
    original_flags: u16,
    weather_code: u8,
    mood_code: u8,
    anniversary: bool,
) -> Result<u16, DayFormatError> {
    if weather_code > 31 {
        return Err(DayFormatError::InvalidWeatherCode);
    }
    if mood_code > 15 {
        return Err(DayFormatError::InvalidMoodCode);
    }

    let reserved = original_flags & 0x003f;
    let weather = u16::from(weather_code) << 11;
    let mood = u16::from(mood_code) << 7;
    let anniversary = if anniversary { 0x0040 } else { 0 };
    Ok(reserved | weather | mood | anniversary)
}

pub fn parse_day_file(bytes: &[u8]) -> Result<DayFile, DayFormatError> {
    if bytes.len() < HEADER_LEN {
        return Err(DayFormatError::TooShort);
    }
    if &bytes[..3] != SIGNATURE {
        return Err(DayFormatError::InvalidSignature);
    }

    let count = usize::from(bytes[3]);
    let declared_len = u32::from_le_bytes(bytes[4..8].try_into().expect("fixed header range"));
    let actual_len = bytes.len() - HEADER_LEN;
    if declared_len as usize != actual_len {
        return Err(DayFormatError::PayloadLengthMismatch {
            declared: declared_len as usize,
            actual: actual_len,
        });
    }

    let body: Vec<u8> = bytes[HEADER_LEN..]
        .iter()
        .map(|byte| byte ^ XOR_KEY)
        .collect();
    let mut records = Vec::with_capacity(count);
    let mut offset = 0;

    for index in 0..count {
        if body.len().saturating_sub(offset) < RECORD_HEADER_LEN {
            return Err(DayFormatError::TruncatedRecordHeader { index });
        }

        let day = body[offset];
        if !(1..=31).contains(&day) {
            return Err(DayFormatError::InvalidDay { index, day });
        }
        let flags = u16::from_le_bytes([body[offset + 1], body[offset + 2]]);
        let text_len = usize::from(u16::from_le_bytes([body[offset + 3], body[offset + 4]]));
        if text_len == 0 {
            return Err(DayFormatError::EmptyTextRecord { index });
        }

        let text_start = offset + RECORD_HEADER_LEN;
        let end = text_start
            .checked_add(text_len)
            .ok_or(DayFormatError::TruncatedRecordText { index })?;
        if end > body.len() {
            return Err(DayFormatError::TruncatedRecordText { index });
        }

        let raw_text = body[text_start..end].to_vec();
        if raw_text.last() != Some(&0) {
            return Err(DayFormatError::MissingTextTerminator { index });
        }
        let (decoded, had_errors) =
            SHIFT_JIS.decode_without_bom_handling(&raw_text[..text_len - 1]);
        let text = decoded.into_owned();
        records.push(DayRecord {
            day,
            flags,
            original_text: Some(text.clone()),
            text,
            original_text_bytes: Some(raw_text),
            decode_had_errors: had_errors,
        });
        offset = end;
    }

    if offset != body.len() {
        return Err(DayFormatError::TrailingBodyBytes {
            count: body.len() - offset,
        });
    }

    Ok(DayFile { records })
}

pub fn encode_day_file(records: &[DayRecord]) -> Result<Vec<u8>, DayFormatError> {
    let count = u8::try_from(records.len()).map_err(|_| DayFormatError::TooManyRecords)?;
    let mut body = Vec::new();

    for (index, record) in records.iter().enumerate() {
        if !(1..=31).contains(&record.day) {
            return Err(DayFormatError::InvalidDay {
                index,
                day: record.day,
            });
        }

        let text_bytes = encoded_text(record)?;
        let text_len =
            u16::try_from(text_bytes.len()).map_err(|_| DayFormatError::TextTooLong { index })?;
        body.push(record.day);
        body.extend_from_slice(&record.flags.to_le_bytes());
        body.extend_from_slice(&text_len.to_le_bytes());
        body.extend_from_slice(&text_bytes);
    }

    let body_len = u32::try_from(body.len()).map_err(|_| DayFormatError::PayloadTooLarge)?;
    let mut output = Vec::with_capacity(HEADER_LEN + body.len());
    output.extend_from_slice(SIGNATURE);
    output.push(count);
    output.extend_from_slice(&body_len.to_le_bytes());
    output.extend(body.into_iter().map(|byte| byte ^ XOR_KEY));
    Ok(output)
}

fn encoded_text(record: &DayRecord) -> Result<Vec<u8>, DayFormatError> {
    if record.original_text.as_deref() == Some(record.text.as_str()) {
        if let Some(raw) = &record.original_text_bytes {
            return Ok(raw.clone());
        }
    }

    if record.text.contains('\0') {
        return Err(DayFormatError::EmbeddedNul);
    }

    let (encoded, _, had_errors) = SHIFT_JIS.encode(&record.text);
    if had_errors {
        return Err(DayFormatError::TextNotRepresentableInShiftJis);
    }

    let mut bytes = encoded.into_owned();
    bytes.push(0);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: [u8; 40] = [
        b'N', b'2', b'0', 2, 0x20, 0, 0, 0, // signature, record count, body size
        0x1b, 0x12, 0x12, 0x19, 0x12, 0x41, 0x4b, 0x5c, 0x46, 0x5a, 0x57, 0x46, 0x5b, 0x51, 0x20,
        0x12, // synthetic day 9 record
        0x1a, 0x12, 0x12, 0x19, 0x12, 0x41, 0x4b, 0x5c, 0x46, 0x5a, 0x57, 0x46, 0x5b, 0x51, 0x23,
        0x12, // synthetic day 8 record
    ];

    #[test]
    fn parses_and_reproduces_synthetic_legacy_fixture() {
        let file = parse_day_file(&FIXTURE).unwrap();
        assert_eq!(file.records.len(), 2);
        assert_eq!(file.records[0].day, 9);
        assert_eq!(file.records[0].text, "SYNTHETIC2");
        assert_eq!(file.records[1].day, 8);
        assert_eq!(file.records[1].text, "SYNTHETIC1");
        assert_eq!(file.records[0].flags, 0);
        assert_eq!(encode_day_file(&file.records).unwrap(), FIXTURE);
    }

    #[test]
    fn packs_metadata_in_observed_bit_positions_and_preserves_reserved_bits() {
        let flags = flags_with_metadata(0x0035, 27, 3, true).unwrap();
        assert_eq!(flags, 0xd9f5);
        let record = DayRecord::new(1, flags, String::new());
        assert_eq!(record.weather_code(), 27);
        assert_eq!(record.mood_code(), 3);
        assert!(record.is_anniversary());
        assert_eq!(flags & 0x003f, 0x0035);
    }

    #[test]
    fn every_original_weather_and_mood_choice_survives_save_and_reload() {
        for weather in 0..=27 {
            for mood in 0..=3 {
                let flags = flags_with_metadata(0x002d, weather, mood, true).unwrap();
                let bytes = encode_day_file(&[DayRecord::new(
                    8,
                    flags,
                    "synthetic metadata fixture".to_owned(),
                )])
                .unwrap();
                let restored = parse_day_file(&bytes).unwrap();
                let record = &restored.records[0];

                assert_eq!(record.weather_code(), weather);
                assert_eq!(record.mood_code(), mood);
                assert!(record.is_anniversary());
                assert_eq!(record.flags & 0x003f, 0x002d);
            }
        }
    }

    #[test]
    fn preserves_unknown_metadata_codes_and_reserved_flags() {
        let flags = flags_with_metadata(0x002d, 31, 15, true).unwrap();
        let bytes = encode_day_file(&[DayRecord::new(
            8,
            flags,
            "synthetic unknown-code fixture".to_owned(),
        )])
        .unwrap();
        let mut record = parse_day_file(&bytes).unwrap().records.remove(0);

        assert_eq!(record.weather_code(), 31);
        assert_eq!(record.mood_code(), 15);
        record.flags = flags_with_metadata(
            record.flags,
            record.weather_code(),
            record.mood_code(),
            false,
        )
        .unwrap();
        let restored = parse_day_file(&encode_day_file(&[record]).unwrap()).unwrap();
        assert_eq!(restored.records[0].weather_code(), 31);
        assert_eq!(restored.records[0].mood_code(), 15);
        assert!(!restored.records[0].is_anniversary());
        assert_eq!(restored.records[0].flags & 0x003f, 0x002d);
    }

    #[test]
    fn round_trips_shift_jis_text_and_metadata() {
        let flags = flags_with_metadata(0, 5, 2, true).unwrap();
        let records = vec![DayRecord::new(12, flags, "日本語の日記".to_owned())];
        let bytes = encode_day_file(&records).unwrap();
        let parsed = parse_day_file(&bytes).unwrap();
        assert_eq!(parsed.records[0].text, "日本語の日記");
        assert_eq!(parsed.records[0].weather_code(), 5);
        assert_eq!(parsed.records[0].mood_code(), 2);
        assert!(parsed.records[0].is_anniversary());
        assert_eq!(encode_day_file(&parsed.records).unwrap(), bytes);
    }

    #[test]
    fn rejects_inconsistent_header_and_missing_text_terminator() {
        let mut bad_length = FIXTURE;
        bad_length[4] = 0x1f;
        assert!(matches!(
            parse_day_file(&bad_length),
            Err(DayFormatError::PayloadLengthMismatch { .. })
        ));

        let mut missing_nul = FIXTURE;
        let last = missing_nul.len() - 1;
        missing_nul[last] ^= 1;
        assert_eq!(
            parse_day_file(&missing_nul),
            Err(DayFormatError::MissingTextTerminator { index: 1 })
        );
    }

    #[test]
    fn rejects_text_that_the_legacy_ansi_writer_cannot_encode() {
        let records = vec![DayRecord::new(1, 0, "emoji 😀".to_owned())];
        assert_eq!(
            encode_day_file(&records),
            Err(DayFormatError::TextNotRepresentableInShiftJis)
        );
    }

    #[test]
    fn reports_text_length_overflow_with_a_specific_error() {
        let records = vec![DayRecord::new(1, 0, "a".repeat(65_535))];
        let error = encode_day_file(&records).unwrap_err();
        assert_eq!(error, DayFormatError::TextTooLong { index: 0 });
        assert!(error.to_string().contains("exceeds 65535 bytes"));
    }
}
