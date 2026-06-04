fn main() {
    cc::Build::new()
        .include("vendor/secsgmlc/src")
        .file("vendor/secsgmlc/src/secsgml.c")
        .file("vendor/secsgmlc/src/uudecode.c")
        .file("vendor/secsgmlc/src/standardize_submission_metadata.c")
        .flag_if_supported("-O3")
        // Vendored C: don't diverge upstream source for benign unused-symbol
        // warnings; silence them at the build level instead.
        .flag_if_supported("-Wno-unused-variable")
        .flag_if_supported("-Wno-unused-const-variable")
        .compile("secsgmlc");

    println!("cargo:rerun-if-changed=vendor/secsgmlc/src/secsgml.c");
    println!("cargo:rerun-if-changed=vendor/secsgmlc/src/secsgml.h");
    println!("cargo:rerun-if-changed=vendor/secsgmlc/src/uudecode.c");
    println!("cargo:rerun-if-changed=vendor/secsgmlc/src/uudecode.h");
    println!("cargo:rerun-if-changed=vendor/secsgmlc/src/standardize_submission_metadata.c");
    println!("cargo:rerun-if-changed=vendor/secsgmlc/src/standardize_submission_metadata.h");
}
