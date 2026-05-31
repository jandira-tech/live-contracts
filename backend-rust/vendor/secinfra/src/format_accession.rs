pub fn format_accession_str(accession: &str, fmt: &str) -> String {
    let n: u64 = accession
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .expect("invalid accession");

    format_accession_int(n, fmt)
}

pub fn format_accession_int(n: u64, fmt: &str) -> String {
    match fmt {
        "int" => n.to_string(),
        "nodash" => format!("{:018}", n),
        "dash" => {
            let s = format!("{:018}", n);
            format!("{}-{}-{}", &s[..10], &s[10..12], &s[12..])
        }
        _ => panic!("invalid format"),
    }
}

#[allow(dead_code)]
pub fn detect_format(s: &str) -> &str {
    if s.contains('-') { "dash" } else { "nodash" }
}
