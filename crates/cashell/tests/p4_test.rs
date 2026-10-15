//! Integration and offline deterministic checks for P4 layer (REQ-P4-01, REQ-P4-02).

use cashell::commands::r2::{build_signed, public_url, sha256_file};
use cashell::commands::vault::{collect_md_files, scan_file};
use sha2::{Digest, Sha256};

#[test]
fn sha256_nist_test_vector_empty_and_fox() {
    // Empty string
    let h1 = Sha256::new();
    let empty_hex: String = h1.finalize().iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        empty_hex,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );

    // "The quick brown fox jumps over the lazy dog"
    let mut h2 = Sha256::new();
    h2.update(b"The quick brown fox jumps over the lazy dog");
    let fox_hex: String = h2.finalize().iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        fox_hex,
        "d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592"
    );
}

#[test]
fn sigv4_canonical_request_exact_layout() {
    let host = "67890abcdef.r2.cloudflarestorage.com";
    let uri = "/my-bucket/dist/app.tar.gz";
    let amz_date = "20261011T120000Z";
    let payload = "d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592";

    let signed = build_signed(
        "PUT",
        host,
        uri,
        "AKIAEXAMPLE123",
        "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
        amz_date,
        payload,
    )
    .expect("sign");

    let expected_canonical = format!(
        "PUT\n{uri}\n\nhost:{host}\nx-amz-content-sha256:{payload}\nx-amz-date:{amz_date}\n\nhost;x-amz-content-sha256;x-amz-date\n{payload}"
    );
    assert_eq!(signed.canonical_request, expected_canonical);
    assert_eq!(
        signed.signed_headers,
        "host;x-amz-content-sha256;x-amz-date"
    );
    assert!(signed
        .string_to_sign
        .starts_with("AWS4-HMAC-SHA256\n20261011T120000Z\n20261011/auto/s3/aws4_request\n"));
    assert!(signed
        .authorization
        .contains("Credential=AKIAEXAMPLE123/20261011/auto/s3/aws4_request"));
    assert!(signed
        .authorization
        .contains("SignedHeaders=host;x-amz-content-sha256;x-amz-date"));
    assert!(signed.authorization.contains("Signature="));
}

#[test]
fn sha256_file_matches_in_memory() {
    let tmp = std::env::temp_dir().join("cashell_p4_test_hash.txt");
    std::fs::write(&tmp, b"The quick brown fox jumps over the lazy dog").unwrap();
    let computed = sha256_file(&tmp).unwrap();
    assert_eq!(
        computed,
        "d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592"
    );
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn public_edge_mirror_format() {
    assert_eq!(
        public_url("my-app", "releases/v1.0.tar.gz"),
        "https://my-app.fath.my.id/releases/v1.0.tar.gz"
    );
}

#[test]
fn vault_query_scans_nested_markdown() {
    let root = std::env::temp_dir().join("cashell_vault_test_tree");
    let _ = std::fs::remove_dir_all(&root);
    let sub = root.join("nested");
    std::fs::create_dir_all(&sub).unwrap();

    let f1 = root.join("a.md");
    let f2 = sub.join("b.md");
    let f3 = root.join("c.txt"); // ignored by md scan

    std::fs::write(&f1, "# Header\nFirst match target here\nFooter").unwrap();
    std::fs::write(&f2, "---\ntitle: test\n---\nSecond TARGET here").unwrap();
    std::fs::write(&f3, "TARGET here but not md").unwrap();

    let files = collect_md_files(&root);
    assert_eq!(files.len(), 2);

    let mut hits = Vec::new();
    for f in &files {
        hits.extend(scan_file(f, &root, "target"));
    }
    assert_eq!(hits.len(), 2);
    assert!(hits
        .iter()
        .any(|h| h.text == "First match target here" && h.line == 2));
    assert!(hits
        .iter()
        .any(|h| h.text == "Second TARGET here" && h.line == 4));

    let _ = std::fs::remove_dir_all(&root);
}
