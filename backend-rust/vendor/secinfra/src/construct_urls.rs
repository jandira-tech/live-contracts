use crate::format_accession::format_accession_int;

pub fn construct_index_url(accession: u64) -> String {
    format!(
        "https://www.sec.gov/Archives/edgar/data/{}/{}-index.html",
        format_accession_int(accession, "nodash"),
        format_accession_int(accession, "dash"),
    )
}

pub fn construct_sgml_url(accession: u64, cik: u64) -> String {
    format!(
        "https://www.sec.gov/Archives/edgar/data/{}/{}/{}.txt",
        cik,
        format_accession_int(accession, "nodash"),
        format_accession_int(accession, "dash"),
    )
}

pub fn construct_folder_url(accession: u64, cik: u64) -> String {
    format!(
        "https://www.sec.gov/Archives/edgar/data/{}/{}/",
        cik,
        format_accession_int(accession, "nodash"),
    )
}

pub fn construct_document_url(accession: u64, cik: u64, filename: &str) -> String {
    format!(
        "https://www.sec.gov/Archives/edgar/data/{}/{}/{}",
        cik,
        format_accession_int(accession, "nodash"),
        filename,
    )
}
