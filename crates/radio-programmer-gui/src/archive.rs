//! The file formats some regulators publish their lists in: a zip archive
//! and a dBASE table inside it, as ISED does. Only what those lists need is
//! read: stored or deflated members of an archive without zip64, and
//! character and number columns of a table.

/// Local file header, central directory entry and end of central directory
/// signatures.
const LOCAL: u32 = 0x0403_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const END: u32 = 0x0605_4b50;
/// The largest member unpacked, in bytes.
const MAX_MEMBER_BYTES: usize = 64 * 1024 * 1024;

fn u16_at(bytes: &[u8], at: usize) -> Option<usize> {
    let pair = bytes.get(at..at + 2)?;
    Some(usize::from(u16::from_le_bytes([pair[0], pair[1]])))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let quad = bytes.get(at..at + 4)?;
    Some(u32::from_le_bytes([quad[0], quad[1], quad[2], quad[3]]))
}

fn usize_at(bytes: &[u8], at: usize) -> Option<usize> {
    usize::try_from(u32_at(bytes, at)?).ok()
}

/// The member of the zip archive `zip` named `name`, ignoring case,
/// unpacked.
pub(crate) fn unzip(zip: &[u8], name: &str) -> Result<Vec<u8>, String> {
    let malformed = || format!("zip archive: malformed looking for {name}");
    // The end record is the last thing in the archive, before a comment of
    // at most 64 KiB.
    let end = (0..=zip.len().saturating_sub(22))
        .rev()
        .take(65_536 + 22)
        .find(|&at| u32_at(zip, at) == Some(END))
        .ok_or("zip archive: no end of central directory")?;
    let count = u16_at(zip, end + 10).ok_or_else(malformed)?;
    let mut entry = usize_at(zip, end + 16).ok_or_else(malformed)?;
    for _ in 0..count {
        if u32_at(zip, entry) != Some(CENTRAL) {
            return Err(malformed());
        }
        let field = |offset| u16_at(zip, entry + offset).ok_or_else(malformed);
        let method = field(10)?;
        let packed = usize_at(zip, entry + 20).ok_or_else(malformed)?;
        let (name_len, extra_len, comment_len) = (field(28)?, field(30)?, field(32)?);
        let local = usize_at(zip, entry + 42).ok_or_else(malformed)?;
        let member = zip
            .get(entry + 46..entry + 46 + name_len)
            .ok_or_else(malformed)?;
        entry += 46 + name_len + extra_len + comment_len;
        if !member.eq_ignore_ascii_case(name.as_bytes()) {
            continue;
        }
        if u32_at(zip, local) != Some(LOCAL) {
            return Err(malformed());
        }
        let start = local
            + 30
            + u16_at(zip, local + 26).ok_or_else(malformed)?
            + u16_at(zip, local + 28).ok_or_else(malformed)?;
        let data = zip.get(start..start + packed).ok_or_else(malformed)?;
        return match method {
            0 => Ok(data.to_vec()),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(data, MAX_MEMBER_BYTES)
                .map_err(|error| format!("zip archive: {name}: {error:?}")),
            other => Err(format!("zip archive: {name}: compression method {other}")),
        };
    }
    Err(format!("zip archive: no {name}"))
}

/// A dBASE table as CSV text: a heading row of the column names, then a row
/// per record not marked deleted, each field trimmed and quoted where it
/// holds a comma, quote or line break. Text is read as Latin-1, as the
/// tables ISED publishes are.
pub(crate) fn dbf_to_csv(dbf: &[u8]) -> Result<String, String> {
    let malformed = || "dBASE table: malformed".to_owned();
    let records = usize_at(dbf, 4).ok_or_else(malformed)?;
    let header = u16_at(dbf, 8).ok_or_else(malformed)?;
    let record = u16_at(dbf, 10).ok_or_else(malformed)?;
    let mut columns = Vec::new();
    let mut at = 32;
    while dbf.get(at).is_some_and(|&byte| byte != 0x0D) && at + 32 <= header {
        let name = dbf.get(at..at + 11).ok_or_else(malformed)?;
        let name: String = name
            .iter()
            .take_while(|&&byte| byte != 0)
            .map(|&byte| char::from(byte))
            .collect();
        let width = usize::from(*dbf.get(at + 16).ok_or_else(malformed)?);
        columns.push((name, width));
        at += 32;
    }
    if columns.is_empty() || record == 0 {
        return Err(malformed());
    }
    let quote = |field: &str| {
        if field.contains([',', '"', '\n', '\r']) {
            format!("\"{}\"", field.replace('"', "\"\""))
        } else {
            field.to_owned()
        }
    };
    let names: Vec<_> = columns.iter().map(|(name, _)| quote(name)).collect();
    let mut csv = names.join(",");
    csv.push('\n');
    for index in 0..records {
        let start = header + index * record;
        let row = dbf.get(start..start + record).ok_or_else(malformed)?;
        if row[0] == b'*' {
            continue;
        }
        let mut fields = Vec::with_capacity(columns.len());
        let mut offset = 1;
        for (_, width) in &columns {
            let bytes = row.get(offset..offset + width).ok_or_else(malformed)?;
            // Latin-1 is the first 256 code points.
            let text: String = bytes.iter().map(|&byte| char::from(byte)).collect();
            fields.push(quote(text.trim()));
            offset += width;
        }
        csv.push_str(&fields.join(","));
        csv.push('\n');
    }
    Ok(csv)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{dbf_to_csv, unzip};

    /// A dBASE table of `columns` (name and width) and `rows`, each row's
    /// fields padded to its column's width; a row led by `*` is deleted.
    pub(crate) fn dbf(columns: &[(&str, u8)], rows: &[(bool, &[&[u8]])]) -> Vec<u8> {
        let header = 32 + 32 * columns.len() + 1;
        let record = 1 + columns.iter().map(|(_, w)| usize::from(*w)).sum::<usize>();
        let mut out = vec![0u8; 32];
        out[0] = 3;
        out[4..8].copy_from_slice(&u32::try_from(rows.len()).unwrap().to_le_bytes());
        out[8..10].copy_from_slice(&u16::try_from(header).unwrap().to_le_bytes());
        out[10..12].copy_from_slice(&u16::try_from(record).unwrap().to_le_bytes());
        for (name, width) in columns {
            let mut descriptor = [0u8; 32];
            descriptor[..name.len()].copy_from_slice(name.as_bytes());
            descriptor[11] = b'C';
            descriptor[16] = *width;
            out.extend_from_slice(&descriptor);
        }
        out.push(0x0D);
        for (deleted, fields) in rows {
            out.push(if *deleted { b'*' } else { b' ' });
            for ((_, width), field) in columns.iter().zip(fields.iter()) {
                let mut padded = field.to_vec();
                padded.resize(usize::from(*width), b' ');
                out.extend_from_slice(&padded);
            }
        }
        out.push(0x1A);
        out
    }

    /// A zip archive of `members`, each deflated when `deflate`, else
    /// stored.
    pub(crate) fn zip(members: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data) in members {
            let packed = if deflate {
                miniz_oxide::deflate::compress_to_vec(data, 6)
            } else {
                data.to_vec()
            };
            let method: u16 = if deflate { 8 } else { 0 };
            let offset = u32::try_from(out.len()).unwrap();
            let sizes = |into: &mut Vec<u8>| {
                into.extend_from_slice(&[0; 4]); // CRC, unchecked here
                into.extend_from_slice(&u32::try_from(packed.len()).unwrap().to_le_bytes());
                into.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
            };
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&[20, 0, 0, 0]);
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0; 4]);
            sizes(&mut out);
            out.extend_from_slice(&u16::try_from(name.len()).unwrap().to_le_bytes());
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&packed);
            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            central.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&[0; 4]);
            sizes(&mut central);
            central.extend_from_slice(&u16::try_from(name.len()).unwrap().to_le_bytes());
            central.extend_from_slice(&[0; 12]);
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let start = u32::try_from(out.len()).unwrap();
        let count = u16::try_from(members.len()).unwrap();
        out.extend_from_slice(&central);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&u32::try_from(central.len()).unwrap().to_le_bytes());
        out.extend_from_slice(&start.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out
    }

    #[test]
    fn a_member_is_found_by_name_stored_or_deflated() {
        for deflate in [false, true] {
            let archive = zip(
                &[("readme.txt", b"hello"), ("FMSTATIO.DBF", b"table")],
                deflate,
            );
            assert_eq!(unzip(&archive, "fmstatio.dbf").unwrap(), b"table");
            assert_eq!(unzip(&archive, "readme.txt").unwrap(), b"hello");
            assert!(unzip(&archive, "missing.dbf").is_err());
        }
        assert!(unzip(b"not a zip", "x").is_err());
        assert!(unzip(&[], "x").is_err());
    }

    #[test]
    fn a_table_reads_as_csv_without_its_deleted_records() {
        let table = dbf(
            &[("CITY", 10), ("CALL_SIGN", 8)],
            &[
                (false, &[b"Montr\xe9al", b"CBF-FM"]),
                (true, &[b"Gone", b"CXXX-FM"]),
                (false, &[b"Comma, ON", b"CA\"B"]),
            ],
        );
        assert_eq!(
            dbf_to_csv(&table).unwrap(),
            "CITY,CALL_SIGN\nMontréal,CBF-FM\n\"Comma, ON\",\"CA\"\"B\"\n"
        );
        assert!(dbf_to_csv(&table[..20]).is_err());
    }
}
