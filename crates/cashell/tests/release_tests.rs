//! Release Readiness & Verification Tests (RL-01, RL-02, RL-03).

use std::path::Path;

#[test]
fn test_rl01_version_synchronization_across_manifests() {
    let pkg_version = env!("CARGO_PKG_VERSION");
    assert_eq!(
        pkg_version, "2.0.1",
        "CAShell Cargo package version must be 2.0.1"
    );

    let workspace_cargo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("Cargo.toml"))
        .expect("workspace Cargo.toml path");

    let content =
        std::fs::read_to_string(&workspace_cargo).expect("must read workspace Cargo.toml");

    assert!(
        content.contains("version = \"2.0.1\""),
        "workspace Cargo.toml must declare version = 2.0.1"
    );
}

#[test]
fn test_rl02_install_service_random_token_and_systemd_hardening() {
    // 1. Hardened system unit inspection
    let unit = cashell::commands::daemon::server::generate_service_unit(false);
    assert!(
        unit.contains("User=cashell"),
        "Hardened system unit must run as unprivileged cashell user"
    );
    assert!(
        unit.contains("NoNewPrivileges=true"),
        "Hardened system unit must have NoNewPrivileges=true"
    );
    assert!(
        unit.contains("ProtectSystem=strict"),
        "Hardened system unit must restrict filesystem write"
    );
    assert!(
        unit.contains("ProtectHome=read-only"),
        "Hardened system unit must protect home directory"
    );

    // 2. Token generation test
    let mut buf = [0u8; 32];
    use rand::RngCore;
    rand::thread_rng().fill_bytes(&mut buf);
    let token = cashell::pro::b64_url_encode_public(&buf);
    assert_eq!(
        token.len(),
        43,
        "Base64url token from 32 bytes should be 43 characters"
    );

    // 3. Sudoers rule generator
    let sudoers = cashell::server::hardening::generate_limited_sudoers("cashell");
    assert!(sudoers.contains("/usr/sbin/ufw"));
    assert!(sudoers.contains("/usr/bin/smartctl"));
    assert!(sudoers.contains("/bin/systemctl restart *"));
}

#[test]
fn test_rl03_readme_free_pro_matrix_and_measured_ram_integrity() {
    let readme_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("README.md"))
        .expect("README.md path");

    let readme = std::fs::read_to_string(&readme_path).expect("must be able to read README.md");

    // Verify Version
    assert!(
        readme.contains("version-2.0.1"),
        "README must display version 2.0.1 badge"
    );
    assert!(
        readme.contains("# CAShell (`cas`) v2.0.1"),
        "README title must display v2.0.1"
    );

    // Verify Measured RAM from V-02 (no unmeasured claims)
    assert!(
        readme.contains("VmRSS 2.2 MB"),
        "README must document measured daemon VmRSS 2.2 MB"
    );
    assert!(
        readme.contains("VmRSS 4.8 MB"),
        "README must document measured Cockpit VmRSS 4.8 MB"
    );
    assert!(
        !readme.contains("<15 MB"),
        "README must not contain unverified '<15 MB' claim"
    );
    assert!(
        !readme.contains("<28 MB"),
        "README must not contain unverified '<28 MB' claim"
    );

    // Verify Free vs Pro Matrix sections
    assert!(
        readme.contains("## 🛡️ Matriks Fitur Free vs Pro"),
        "README must include Free vs Pro matrix"
    );
    assert!(readme.contains("Cockpit multi-node fleet (>1 server via `caorch`)"));
    assert!(readme.contains("Auto-ban SSH brute-force login"));
    assert!(readme.contains("Rolling update zero-downtime & instant rollback"));
    assert!(readme.contains("Zero-knowledge end-to-end encrypted sync"));
}
