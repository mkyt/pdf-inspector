//! Text strings and dates.

/// Decode a text string: UTF-16BE or UTF-8 with BOM, PDFDocEncoding otherwise
/// (approximated by Latin-1 plus the 0x80..0x9f punctuation block).
pub fn decode(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xfe, 0xff]) {
        let units: Vec<u16> = rest
            .chunks(2)
            .map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    bytes.iter().map(|&b| pdfdoc_char(b)).collect()
}

fn pdfdoc_char(b: u8) -> char {
    const HIGH: [char; 32] = [
        '•', '†', '‡', '…', '—', '–', 'ƒ', '⁄', '‹', '›', '−', '‰', '„', '“', '”', '‘', '’', '‚',
        '™', 'ﬁ', 'ﬂ', 'Ł', 'Œ', 'Š', 'Ÿ', 'Ž', 'ı', 'ł', 'œ', 'š', 'ž', '\u{fffd}',
    ];
    match b {
        0x80..=0x9f => HIGH[(b - 0x80) as usize],
        0xa0 => '€',
        _ => b as char,
    }
}

/// A date as written in a PDF date string or an XMP date, with the fields
/// that were given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Date {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// Offset from UTC in minutes; `None` when the time zone is unknown.
    pub offset: Option<i16>,
    /// Number of fields given: 1 (year) to 6 (seconds).
    pub precision: u8,
}

impl Date {
    /// ISO 8601, with as many fields as were given.
    pub fn iso(&self) -> String {
        let mut s = format!("{:04}", self.year);
        if self.precision >= 2 {
            s += &format!("-{:02}", self.month);
        }
        if self.precision >= 3 {
            s += &format!("-{:02}", self.day);
        }
        if self.precision >= 4 {
            s += &format!("T{:02}:{:02}", self.hour, self.minute);
            if self.precision >= 6 {
                s += &format!(":{:02}", self.second);
            }
            match self.offset {
                Some(0) => s.push('Z'),
                Some(o) => {
                    s += &format!(
                        "{}{:02}:{:02}",
                        if o < 0 { '-' } else { '+' },
                        o.unsigned_abs() / 60,
                        o.unsigned_abs() % 60
                    )
                }
                None => {}
            }
        }
        s
    }

    /// Seconds since 1970 in UTC (local time when the zone is unknown).
    fn instant(&self) -> i64 {
        let (y, m) = if self.month <= 2 {
            (self.year as i64 - 1, self.month as i64 + 9)
        } else {
            (self.year as i64, self.month as i64 - 3)
        };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * m + 2) / 5 + self.day as i64 - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        days * 86_400 + self.hour as i64 * 3600 + self.minute as i64 * 60 + self.second as i64
            - self.offset.unwrap_or(0) as i64 * 60
    }

    /// Whether both denote the same time, as far as both are precise. Dates
    /// without a time zone match any zone.
    pub fn same_as(&self, o: &Date) -> bool {
        let p = self.precision.min(o.precision);
        if p < 4 || self.offset.is_none() || o.offset.is_none() {
            let a = (
                self.year,
                self.month,
                self.day,
                self.hour,
                self.minute,
                self.second,
            );
            let b = (o.year, o.month, o.day, o.hour, o.minute, o.second);
            return fields(a, p) == fields(b, p);
        }
        let truncate = |d: &Date| {
            let mut d = *d;
            if p < 6 {
                d.second = 0;
            }
            d.instant()
        };
        truncate(self) == truncate(o)
    }
}

fn fields(f: (i32, u8, u8, u8, u8, u8), n: u8) -> [i32; 6] {
    let all = [
        f.0, f.1 as i32, f.2 as i32, f.3 as i32, f.4 as i32, f.5 as i32,
    ];
    let mut out = [0; 6];
    out[..n as usize].copy_from_slice(&all[..n as usize]);
    out
}

fn digits(s: &[u8], at: usize, n: usize) -> Option<u32> {
    let d = s.get(at..at + n)?;
    d.iter()
        .all(u8::is_ascii_digit)
        .then(|| d.iter().fold(0u32, |acc, &c| acc * 10 + (c - b'0') as u32))
}

fn valid(d: Date) -> Option<Date> {
    let ok = (1..=12).contains(&d.month)
        && (1..=31).contains(&d.day)
        && d.hour < 24
        && d.minute < 60
        && d.second < 60;
    ok.then_some(d)
}

/// Parse a PDF date: `D:YYYYMMDDHHmmSSOHH'mm` (everything after the year
/// optional, `D:` too). Returns `None` for anything else.
pub fn pdf_date(s: &[u8]) -> Option<Date> {
    let s = s.strip_prefix(b"D:").unwrap_or(s);
    let mut d = Date {
        year: digits(s, 0, 4)? as i32,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
        offset: None,
        precision: 1,
    };
    let mut at = 4;
    for (i, f) in [
        &mut d.month,
        &mut d.day,
        &mut d.hour,
        &mut d.minute,
        &mut d.second,
    ]
    .into_iter()
    .enumerate()
    {
        match digits(s, at, 2) {
            Some(v) => {
                *f = v as u8;
                at += 2;
                d.precision = i as u8 + 2;
            }
            None => break,
        }
    }
    match s.get(at) {
        None => {}
        // A time zone follows a time.
        Some(_) if d.precision < 4 => return None,
        Some(b'Z') => d.offset = Some(0),
        Some(&c @ (b'+' | b'-')) => {
            let h = digits(s, at + 1, 2)? as i16;
            let mut rest = at + 3;
            if s.get(rest) == Some(&b'\'') {
                rest += 1;
            }
            let m = digits(s, rest, 2).unwrap_or(0) as i16;
            let o = h * 60 + m;
            d.offset = Some(if c == b'-' { -o } else { o });
        }
        Some(_) => return None,
    }
    valid(d)
}

/// Parse an XMP (ISO 8601 subset) date: `YYYY[-MM[-DD[Thh:mm[:ss[.s+]][TZD]]]]`.
pub fn xmp_date(s: &str) -> Option<Date> {
    let s = s.trim().as_bytes();
    let mut d = Date {
        year: digits(s, 0, 4)? as i32,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
        offset: None,
        precision: 1,
    };
    let mut at = 4;
    let field = |at: usize, sep: u8| {
        (s.get(at) == Some(&sep))
            .then(|| digits(s, at + 1, 2))
            .flatten()
    };
    if let Some(m) = field(at, b'-') {
        d.month = m as u8;
        d.precision = 2;
        at += 3;
        if let Some(v) = field(at, b'-') {
            d.day = v as u8;
            d.precision = 3;
            at += 3;
            if let (Some(h), Some(mi)) = (field(at, b'T'), field(at + 3, b':')) {
                d.hour = h as u8;
                d.minute = mi as u8;
                d.precision = 5;
                at += 6;
                if let Some(sec) = field(at, b':') {
                    d.second = sec as u8;
                    d.precision = 6;
                    at += 3;
                    if s.get(at) == Some(&b'.') {
                        at += 1;
                        while s.get(at).is_some_and(u8::is_ascii_digit) {
                            at += 1;
                        }
                    }
                }
                match s.get(at) {
                    None => {}
                    Some(b'Z') => d.offset = Some(0),
                    Some(&c @ (b'+' | b'-')) => {
                        let h = digits(s, at + 1, 2)? as i16;
                        let m = field(at + 3, b':').unwrap_or(0) as i16;
                        let o = h * 60 + m;
                        d.offset = Some(if c == b'-' { -o } else { o });
                    }
                    Some(_) => return None,
                }
            }
        }
    }
    valid(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_strings() {
        assert_eq!(decode(b"\xfe\xff\x00A\x00B"), "AB");
        assert_eq!(decode(b"\xef\xbb\xbf\xe3\x81\x82"), "あ");
        assert_eq!(decode(b"a\x84b"), "a—b");
    }

    #[test]
    fn pdf_dates() {
        let d = pdf_date(b"D:20240131123456+09'00'").unwrap();
        assert_eq!(d.iso(), "2024-01-31T12:34:56+09:00");
        assert_eq!(pdf_date(b"D:2024").unwrap().iso(), "2024");
        assert_eq!(pdf_date(b"D:20240131").unwrap().iso(), "2024-01-31");
        assert_eq!(
            pdf_date(b"D:19991231235959Z").unwrap().iso(),
            "1999-12-31T23:59:59Z"
        );
        assert_eq!(
            pdf_date(b"D:20240131120000-05'30").unwrap().offset,
            Some(-330)
        );
        assert!(pdf_date(b"2024-01-31").is_none());
        assert!(pdf_date(b"D:20241331").is_none());
    }

    #[test]
    fn xmp_dates() {
        let x = xmp_date("2024-01-31T03:34:56Z").unwrap();
        let p = pdf_date(b"D:20240131123456+09'00'").unwrap();
        assert!(x.same_as(&p));
        assert_eq!(xmp_date("2024-01-31T12:34:56.123+09:00").unwrap(), p);
        assert!(!xmp_date("2024-01-31T12:34:57+09:00").unwrap().same_as(&p));
        assert!(xmp_date("2024-01").unwrap().same_as(&p));
        assert_eq!(xmp_date("2024-01-31T12:34").unwrap().precision, 5);
    }
}
