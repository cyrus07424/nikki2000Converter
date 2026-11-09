# Legacy reference converter; it does not implement the verified N20 layout.
import os
import csv
import re

DATA_DIR = r'DATA'
OUTPUT_CSV = r'diary.csv'
XOR_KEY = 0x12

def parse_flags(flags_bytes):
    # FIXME
    weather_map = {1: '晴れ', 2: '曇り', 3: '雨', 4: '雪', 5: '晴れ/雨', 6: '晴れ/雨', 7: '曇り/晴れ', 8: '曇り/雨', 9: '雨/晴れ', 10: '雨/曇り', 11: '晴れ→曇り', 12: '晴→雨', 13: '曇り→晴れ', 14: '曇り→雨', 15: '雨→晴れ', 16: '雨→曇り', 17: '雪/晴れ', 18: '雪/曇り', 19: '雪/雨', 20: '晴れ/雪', 21: '曇り/雪', 22: '雨/雪', 23: '雪→晴れ', 24: '雪→曇り', 25: '雪→雨', 26: '晴れ→雪', 27: '曇→雪', 28: '雨→雪'}
    mood_map = {1: '好調', 2: '普通', 3: '不調', 4: '最悪'}

    weather_id = flags_bytes[0] & 0x0F
    mood_id = (flags_bytes[0] >> 4) & 0x0F
    is_anniversary = (flags_bytes[1] & 0x01) != 0

    weather = weather_map.get(weather_id, '不明')
    mood = mood_map.get(mood_id, '不明')

    return weather, mood, is_anniversary

def clean_text(text):
    return re.sub(r'[\x00-\x08\x0b\x0c\x0e-\x1f]', '', text)

def decode_and_split_entries(decoded_body):
    entries = []
    entry_header_pattern = re.compile(b'\x00([\x01-\x1f])([\x00-\xff])([\x00-\xff])([\x00-\xff])([\x00-\xff])')

    last_end = 0
    day_of_first_entry = 1

    matches = list(entry_header_pattern.finditer(decoded_body))

    if not matches:
        text = decoded_body.decode('shift-jis', errors='ignore').strip()
        if text:
            entries.append({'day': day_of_first_entry, 'weather': '不明', 'mood': '不明', 'is_anniversary': False, 'text': clean_text(text)})
        return entries

    # Text before the first header is for day 1
    first_match_start = matches[0].start()
    text = decoded_body[:first_match_start].decode('shift-jis', errors='ignore').strip()
    if text:
        entries.append({'day': day_of_first_entry, 'weather': '不明', 'mood': '不明', 'is_anniversary': False, 'text': clean_text(text)})

    for i, match in enumerate(matches):
        day = match.group(1)[0]
        flags = match.group(2) + match.group(3)
        weather, mood, is_anniversary = parse_flags(flags)

        text_start = match.end()
        text_end = matches[i + 1].start() if i + 1 < len(matches) else len(decoded_body)

        text = decoded_body[text_start:text_end].decode('shift-jis', errors='ignore').strip()
        
        entries.append({'day': day, 'weather': weather, 'mood': mood, 'is_anniversary': is_anniversary, 'text': clean_text(text)})

    # Sort entries by day
    entries.sort(key=lambda x: x['day'])
    return entries


with open(OUTPUT_CSV, 'w', newline='', encoding='utf-8-sig') as csvfile:
    csv_writer = csv.writer(csvfile, quoting=csv.QUOTE_ALL)
    csv_writer.writerow(['date', 'weather', 'mood', 'is_anniversary', 'entry'])

    for filename in sorted(os.listdir(DATA_DIR)):
        if filename.endswith('.DAY'):
            file_path = os.path.join(DATA_DIR, filename)
            date_str = os.path.splitext(filename)[0]
            year = date_str[:4]
            month = int(date_str[4:6])

            with open(file_path, 'rb') as f:
                content = f.read()

            header_size = 13
            if len(content) <= header_size:
                continue

            encoded_body = content[header_size:]
            decoded_body = bytearray([b ^ XOR_KEY for b in encoded_body])

            decoded_entries = decode_and_split_entries(decoded_body)
            
            for entry in decoded_entries:
                if entry['text']:
                    date_formatted = f"{year}-{month:02d}-{entry['day']:02d}"
                    entry_text_cleaned = entry['text'].replace('\r\n', '\\n').replace('\r', '\\n').replace('\n', '\\n')
                    csv_writer.writerow([date_formatted, entry['weather'], entry['mood'], entry['is_anniversary'], entry_text_cleaned])

print(f"Successfully converted all .DAY files to {OUTPUT_CSV}")