#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod day;

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use day::{encode_day_file, flags_with_metadata, parse_day_file, DayRecord};
use eframe::egui;
use rfd::FileDialog;
use tempfile::NamedTempFile;

#[derive(Clone)]
struct MonthFile {
    year: u16,
    month: u8,
    records: Vec<DayRecord>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct EntryKey {
    month_index: usize,
    record_index: usize,
}

#[derive(Clone)]
struct EntryDraft {
    text: String,
    weather_code: u8,
    mood_code: u8,
    anniversary: bool,
}

struct NikkiApp {
    data_dir: PathBuf,
    months: Vec<MonthFile>,
    errors: Vec<String>,
    selected: Option<EntryKey>,
    pending_new: Option<(u16, u8, u8)>,
    draft: Option<EntryDraft>,
    dirty: bool,
    search: String,
    new_year: u16,
    new_month: u8,
    new_day: u8,
    status: String,
}

impl NikkiApp {
    fn new() -> Self {
        let base = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let data_dir = base.join("DATA");
        let (months, errors) = load_months(&data_dir);
        let (new_year, new_month) = months
            .last()
            .map(|month| (month.year, month.month))
            .unwrap_or((2000, 1));
        Self {
            data_dir,
            months,
            errors,
            selected: None,
            pending_new: None,
            draft: None,
            dirty: false,
            search: String::new(),
            new_year,
            new_month,
            new_day: 1,
            status: String::new(),
        }
    }

    fn reload(&mut self, preferred: Option<(u16, u8, u8)>) {
        let (months, errors) = load_months(&self.data_dir);
        self.months = months;
        self.errors = errors;
        self.selected = preferred.and_then(|(year, month, day)| {
            self.months
                .iter()
                .enumerate()
                .find_map(|(month_index, file)| {
                    if file.year != year || file.month != month {
                        return None;
                    }
                    file.records
                        .iter()
                        .position(|record| record.day == day)
                        .map(|record_index| EntryKey {
                            month_index,
                            record_index,
                        })
                })
        });
        self.pending_new = None;
        self.draft = self.selected.map(|key| {
            let record = &self.months[key.month_index].records[key.record_index];
            EntryDraft {
                text: record.text.clone(),
                weather_code: record.weather_code(),
                mood_code: record.mood_code(),
                anniversary: record.is_anniversary(),
            }
        });
        self.dirty = false;
        self.status = format!(
            "読み込み完了: {} 件{}",
            self.months
                .iter()
                .map(|month| month.records.len())
                .sum::<usize>(),
            if self.errors.is_empty() {
                String::new()
            } else {
                format!(" / 読み込みエラー {} 件", self.errors.len())
            }
        );
    }

    fn select_record(&mut self, key: EntryKey) {
        let record = &self.months[key.month_index].records[key.record_index];
        self.draft = Some(EntryDraft {
            text: record.text.clone(),
            weather_code: record.weather_code(),
            mood_code: record.mood_code(),
            anniversary: record.is_anniversary(),
        });
        self.selected = Some(key);
        self.pending_new = None;
        self.dirty = false;
    }

    fn begin_new_entry(&mut self) {
        let (year, month, day) = (self.new_year, self.new_month, self.new_day);
        if !valid_date(year, month, day) {
            self.status = "日付が正しくありません".to_owned();
            return;
        }

        if let Some(key) = self
            .months
            .iter()
            .enumerate()
            .find_map(|(month_index, file)| {
                if file.year != year || file.month != month {
                    return None;
                }
                file.records
                    .iter()
                    .position(|record| record.day == day)
                    .map(|record_index| EntryKey {
                        month_index,
                        record_index,
                    })
            })
        {
            self.select_record(key);
            self.status = "その日付の記録は既にあります".to_owned();
            return;
        }

        self.selected = None;
        self.pending_new = Some((year, month, day));
        self.draft = Some(EntryDraft {
            text: String::new(),
            weather_code: 0,
            mood_code: 0,
            anniversary: false,
        });
        self.dirty = false;
        self.status = "新しい記録です。保存するまでファイルは変更されません".to_owned();
    }

    fn discard_draft(&mut self) {
        if self.pending_new.is_some() {
            self.pending_new = None;
            self.draft = None;
            self.dirty = false;
        } else if let Some(key) = self.selected {
            self.select_record(key);
        }
    }

    fn save_draft(&mut self) {
        let Some(draft) = self.draft.clone() else {
            return;
        };
        let (year, month, day, existing_key) = if let Some((year, month, day)) = self.pending_new {
            (year, month, day, None)
        } else if let Some(key) = self.selected {
            let file = &self.months[key.month_index];
            (
                file.year,
                file.month,
                file.records[key.record_index].day,
                Some(key),
            )
        } else {
            return;
        };

        if existing_key.is_none()
            && draft.text.is_empty()
            && draft.weather_code == 0
            && draft.mood_code == 0
            && !draft.anniversary
        {
            self.status = "旧アプリと同様、本文も属性も空の新規日記は保存しません".to_owned();
            return;
        }

        let path = self.data_dir.join(format!("{year:04}{month:02}.DAY"));
        let loaded_month = self
            .months
            .iter()
            .find(|file| file.year == year && file.month == month);
        if loaded_month.is_none() && path.exists() {
            self.status = "既存の月ファイルを正常に読み込めないため、上書きしません".to_owned();
            return;
        }
        let mut records = loaded_month
            .map(|file| file.records.clone())
            .unwrap_or_default();

        let flags = if let Some(key) = existing_key {
            let record = &mut records[key.record_index];
            match flags_with_metadata(
                record.flags,
                draft.weather_code,
                draft.mood_code,
                draft.anniversary,
            ) {
                Ok(flags) => flags,
                Err(error) => {
                    self.status = error.to_string();
                    return;
                }
            }
        } else {
            match flags_with_metadata(0, draft.weather_code, draft.mood_code, draft.anniversary) {
                Ok(flags) => flags,
                Err(error) => {
                    self.status = error.to_string();
                    return;
                }
            }
        };

        if let Some(key) = existing_key {
            records[key.record_index].flags = flags;
            records[key.record_index].text = draft.text;
        } else {
            records.push(DayRecord::new(day, flags, draft.text));
        }

        let bytes = match encode_day_file(&records) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        if let Err(error) = write_atomic(&path, &bytes) {
            self.status = format!("保存できませんでした: {error}");
            return;
        }

        self.new_year = year;
        self.new_month = month;
        self.new_day = day;
        self.reload(Some((year, month, day)));
        self.status = format!("{year:04}-{month:02}-{day:02} を保存しました");
    }

    fn choose_data_folder(&mut self) {
        if let Some(path) = FileDialog::new()
            .set_title("DATA フォルダーを選択")
            .pick_folder()
        {
            self.data_dir = path;
            self.reload(None);
        }
    }

    fn export_csv(&mut self) {
        let Some(path) = FileDialog::new()
            .add_filter("CSV", &["csv"])
            .set_file_name("nikki2000-export.csv")
            .save_file()
        else {
            return;
        };

        match write_csv_file(&path, &self.months) {
            Ok(()) => self.status = format!("CSV を {} に出力しました", path.display()),
            Err(error) => self.status = format!("CSV を出力できませんでした: {error}"),
        }
    }

    fn visible_rows(&self) -> Vec<(EntryKey, String)> {
        let query = self.search.to_lowercase();
        let mut rows = Vec::new();
        for (month_index, month) in self.months.iter().enumerate() {
            for (record_index, record) in month.records.iter().enumerate() {
                if !query.is_empty() && !record.text.to_lowercase().contains(&query) {
                    continue;
                }
                let preview: String = record
                    .text
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .chars()
                    .take(32)
                    .collect();
                let anniversary = if record.is_anniversary() { " ★" } else { "" };
                let label = format!(
                    "{:04}-{:02}-{:02}  天気{} 気分{}{}  {}",
                    month.year,
                    month.month,
                    record.day,
                    record.weather_code(),
                    record.mood_code(),
                    anniversary,
                    preview
                );
                rows.push((
                    EntryKey {
                        month_index,
                        record_index,
                    },
                    label,
                ));
            }
        }
        rows.sort_by_key(|(key, _)| {
            let month = &self.months[key.month_index];
            (
                month.year,
                month.month,
                month.records[key.record_index].day,
                key.record_index,
            )
        });
        rows
    }
}

impl eframe::App for NikkiApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        let mut choose_folder = false;
        let mut reload = false;
        let mut export = false;
        let mut begin_new = false;

        egui::TopBottomPanel::top("toolbar").show(context, |ui| {
            ui.horizontal(|ui| {
                ui.label("DATA:");
                ui.label(self.data_dir.display().to_string());
                if ui
                    .add_enabled(!self.dirty, egui::Button::new("フォルダーを選択"))
                    .clicked()
                {
                    choose_folder = true;
                }
                if ui
                    .add_enabled(!self.dirty, egui::Button::new("再読み込み"))
                    .clicked()
                {
                    reload = true;
                }
                if ui
                    .add_enabled(
                        !self.dirty && !self.months.is_empty(),
                        egui::Button::new("CSV エクスポート"),
                    )
                    .clicked()
                {
                    export = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("検索:");
                ui.text_edit_singleline(&mut self.search);
                ui.separator();
                ui.label("新規日付:");
                ui.add(
                    egui::DragValue::new(&mut self.new_year)
                        .range(1..=9999)
                        .prefix("年 "),
                );
                ui.add(
                    egui::DragValue::new(&mut self.new_month)
                        .range(1..=12)
                        .prefix("月 "),
                );
                ui.add(
                    egui::DragValue::new(&mut self.new_day)
                        .range(1..=31)
                        .prefix("日 "),
                );
                if ui
                    .add_enabled(!self.dirty, egui::Button::new("新しい日記"))
                    .clicked()
                {
                    begin_new = true;
                }
            });
            if self.dirty {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    "未保存の変更があります。保存または破棄するまで別の日付へ移動できません。",
                );
            }
            if !self.errors.is_empty() {
                ui.collapsing("読み込みエラー", |ui| {
                    for error in &self.errors {
                        ui.label(error);
                    }
                });
            }
        });

        if choose_folder {
            self.choose_data_folder();
        }
        if reload {
            self.reload(None);
        }
        if export {
            self.export_csv();
        }
        if begin_new {
            self.begin_new_entry();
        }

        let rows = self.visible_rows();
        let mut selected_row = None;
        egui::SidePanel::left("entry-list")
            .default_width(360.0)
            .show(context, |ui| {
                ui.heading("日記一覧");
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (key, label) in rows {
                        let enabled = !self.dirty;
                        let selected = self.selected == Some(key);
                        let response = ui
                            .add_enabled_ui(enabled, |ui| ui.selectable_label(selected, label))
                            .inner;
                        if response.clicked() {
                            selected_row = Some(key);
                        }
                    }
                });
            });
        if let Some(key) = selected_row {
            self.select_record(key);
        }

        let mut save = false;
        let mut discard = false;
        egui::CentralPanel::default().show(context, |ui| {
            if let Some(draft) = self.draft.as_mut() {
                let date = if let Some((year, month, day)) = self.pending_new {
                    (year, month, day)
                } else if let Some(key) = self.selected {
                    let file = &self.months[key.month_index];
                    (file.year, file.month, file.records[key.record_index].day)
                } else {
                    return;
                };
                ui.heading(format!("{:04}-{:02}-{:02}", date.0, date.1, date.2));

                if let Some(key) = self.selected {
                    let record = &self.months[key.month_index].records[key.record_index];
                    if record.decode_had_errors {
                        ui.colored_label(
                            egui::Color32::YELLOW,
                            "本文に Shift-JIS として解釈できないバイトがあります。未編集なら元バイトを保持します。",
                        );
                    }
                }

                ui.horizontal(|ui| {
                    ui.label("天気コード");
                    if ui
                        .add(egui::DragValue::new(&mut draft.weather_code).range(0..=31))
                        .changed()
                    {
                        self.dirty = true;
                    }
                    ui.label("気分コード");
                    if ui
                        .add(egui::DragValue::new(&mut draft.mood_code).range(0..=15))
                        .changed()
                    {
                        self.dirty = true;
                    }
                    if ui
                        .checkbox(&mut draft.anniversary, "記念日")
                        .changed()
                    {
                        self.dirty = true;
                    }
                });
                ui.label("天気・気分の表示名は未確定のため、保存値をコードとして扱います。");
                ui.separator();
                if ui
                    .add(
                        egui::TextEdit::multiline(&mut draft.text)
                            .desired_rows(24)
                            .desired_width(f32::INFINITY),
                    )
                    .changed()
                {
                    self.dirty = true;
                }
                ui.horizontal(|ui| {
                    let can_save = self.pending_new.is_some() || self.dirty;
                    if ui
                        .add_enabled(can_save, egui::Button::new("保存 (.DAY)"))
                        .clicked()
                    {
                        save = true;
                    }
                    if ui
                        .add_enabled(self.dirty, egui::Button::new("変更を破棄"))
                        .clicked()
                    {
                        discard = true;
                    }
                });
            } else {
                ui.heading("日記帳2000");
                ui.label("一覧から日記を選択するか、新しい日付を作成してください。");
            }
        });

        if save {
            self.save_draft();
        }
        if discard {
            self.discard_draft();
        }

        egui::TopBottomPanel::bottom("status").show(context, |ui| {
            ui.label(&self.status);
        });
    }
}

fn load_months(data_dir: &Path) -> (Vec<MonthFile>, Vec<String>) {
    if !data_dir.exists() {
        return (Vec::new(), Vec::new());
    }

    let directory = match fs::read_dir(data_dir) {
        Ok(directory) => directory,
        Err(_) => {
            return (
                Vec::new(),
                vec!["DATA フォルダーを読み込めません".to_owned()],
            )
        }
    };

    let mut months = Vec::new();
    let mut errors = Vec::new();
    for item in directory {
        let item = match item {
            Ok(item) => item,
            Err(_) => {
                errors.push("フォルダー内の項目を読み取れません".to_owned());
                continue;
            }
        };
        let path = item.path();
        if !path.is_file()
            || !path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("DAY"))
        {
            continue;
        }

        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default();
        if stem.len() != 6 || !stem.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let year = match stem[..4].parse::<u16>() {
            Ok(year) => year,
            Err(_) => continue,
        };
        let month = match stem[4..].parse::<u8>() {
            Ok(month) if (1..=12).contains(&month) => month,
            _ => {
                errors.push(format!("{}: 月の値が不正です", path.display()));
                continue;
            }
        };

        let parsed = fs::read(&path)
            .map_err(|_| "ファイルを読み込めません".to_owned())
            .and_then(|bytes| parse_day_file(&bytes).map_err(|error| error.to_string()));
        match parsed {
            Ok(file) => months.push(MonthFile {
                year,
                month,
                records: file.records,
            }),
            Err(error) => errors.push(format!("{}: {error}", path.display())),
        }
    }

    months.sort_by_key(|file| (file.year, file.month));
    (months, errors)
}

fn valid_date(year: u16, month: u8, day: u8) -> bool {
    if year == 0 || !(1..=12).contains(&month) {
        return false;
    }
    day >= 1 && day <= days_in_month(year, month)
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year % 400 == 0 || (year % 4 == 0 && year % 100 != 0) => 29,
        2 => 28,
        _ => 31,
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "target has no parent folder")
    })?;
    fs::create_dir_all(parent)?;
    let mut temporary = NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn write_csv_file(path: &Path, months: &[MonthFile]) -> Result<(), Box<dyn std::error::Error>> {
    let mut output = fs::File::create(path)?;
    output.write_all(&[0xef, 0xbb, 0xbf])?;
    let mut writer = csv::WriterBuilder::new()
        .quote_style(csv::QuoteStyle::Always)
        .from_writer(output);
    writer.write_record([
        "date",
        "weather_code",
        "mood_code",
        "is_anniversary",
        "entry",
    ])?;

    let mut entries: Vec<_> = months
        .iter()
        .flat_map(|file| {
            file.records
                .iter()
                .map(move |record| (file.year, file.month, record))
        })
        .collect();
    entries.sort_by_key(|(year, month, record)| (*year, *month, record.day));
    for (year, month, record) in entries {
        writer.write_record([
            format!("{year:04}-{month:02}-{:02}", record.day),
            record.weather_code().to_string(),
            record.mood_code().to_string(),
            record.is_anniversary().to_string(),
            record.text.clone(),
        ])?;
    }
    writer.flush()?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn configure_japanese_font(context: &egui::Context) -> bool {
    let windows_dir = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let font_dir = windows_dir.join("Fonts");
    let font_data = ["YuGothM.ttc", "meiryo.ttc", "msgothic.ttc"]
        .iter()
        .find_map(|name| fs::read(font_dir.join(name)).ok());
    let Some(font_data) = font_data else {
        return false;
    };

    let font_name = "windows-japanese".to_owned();
    let mut definitions = egui::FontDefinitions::default();
    definitions.font_data.insert(
        font_name.clone(),
        std::sync::Arc::new(egui::FontData::from_owned(font_data)),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        if let Some(fonts) = definitions.families.get_mut(&family) {
            fonts.insert(0, font_name.clone());
        }
    }
    context.set_fonts(definitions);
    true
}

#[cfg(not(target_os = "windows"))]
fn configure_japanese_font(_context: &egui::Context) -> bool {
    true
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([820.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "日記帳2000 Rust",
        options,
        Box::new(|context| {
            let mut app = NikkiApp::new();
            if !configure_japanese_font(&context.egui_ctx) {
                app.status = "Windows の日本語フォントを読み込めませんでした".to_owned();
            }
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_new_entry_is_not_written() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("202611.DAY");
        let mut app = NikkiApp {
            data_dir: directory.path().to_path_buf(),
            months: Vec::new(),
            errors: Vec::new(),
            selected: None,
            pending_new: Some((2026, 11, 8)),
            draft: Some(EntryDraft {
                text: String::new(),
                weather_code: 0,
                mood_code: 0,
                anniversary: false,
            }),
            dirty: false,
            search: String::new(),
            new_year: 2026,
            new_month: 11,
            new_day: 8,
            status: String::new(),
        };

        app.save_draft();

        assert!(!path.exists());
        assert!(app.status.contains("保存しません"));
    }

    #[test]
    fn atomic_save_replaces_existing_day_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("202611.DAY");
        fs::write(&path, b"old bytes").unwrap();
        write_atomic(&path, b"new bytes").unwrap();
        assert_eq!(fs::read(path).unwrap(), b"new bytes");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn gregorian_month_lengths_match_legacy_navigation() {
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2025, 2), 28);
        assert!(!valid_date(2025, 2, 29));
    }

    #[test]
    fn bad_format_errors_do_not_include_diary_text() {
        let error = day::DayFormatError::InvalidSignature.to_string();
        assert!(!error.contains("SYNTHETIC"));
    }

    #[test]
    fn csv_export_contains_dates_codes_and_quoted_text() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("export.csv");
        let record = DayRecord::new(
            8,
            flags_with_metadata(0, 3, 2, true).unwrap(),
            "a,\"b\"".to_owned(),
        );
        let month = MonthFile {
            year: 2026,
            month: 11,
            records: vec![record],
        };
        write_csv_file(&path, &[month]).unwrap();
        let bytes = fs::read(path).unwrap();
        assert!(bytes.starts_with(&[0xef, 0xbb, 0xbf]));
        let mut reader = csv::Reader::from_reader(&bytes[3..]);
        let headers = reader.headers().unwrap().clone();
        assert_eq!(
            headers.iter().collect::<Vec<_>>(),
            [
                "date",
                "weather_code",
                "mood_code",
                "is_anniversary",
                "entry"
            ]
        );
        let row = reader.records().next().unwrap().unwrap();
        assert_eq!(&row[0], "2026-11-08");
        assert_eq!(&row[1], "3");
        assert_eq!(&row[2], "2");
        assert_eq!(&row[3], "true");
        assert_eq!(&row[4], "a,\"b\"");
    }
}
