fn main() {
    cc::Build::new()
        .include("vendor/secsgmlc/src")
        .file("vendor/secsgmlc/src/secsgml.c")
        .file("vendor/secsgmlc/src/uudecode.c")
        .file("vendor/secsgmlc/src/standardize_submission_metadata.c")
        .flag_if_supported("-O3")
        .compile("secsgmlc");

    println!("cargo:rerun-if-changed=vendor/secsgmlc/src/secsgml.c");
    println!("cargo:rerun-if-changed=vendor/secsgmlc/src/secsgml.h");
    println!("cargo:rerun-if-changed=vendor/secsgmlc/src/uudecode.c");
    println!("cargo:rerun-if-changed=vendor/secsgmlc/src/uudecode.h");
    println!("cargo:rerun-if-changed=vendor/secsgmlc/src/standardize_submission_metadata.c");
    println!("cargo:rerun-if-changed=vendor/secsgmlc/src/standardize_submission_metadata.h");
}
