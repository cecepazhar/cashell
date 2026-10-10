//! Config round-trip self-check. Run: cargo test

#[cfg(test)]
mod tests {
    use cashell::config::Config;

    #[test]
    fn config_roundtrip() {
        let dir = std::env::temp_dir().join("cashell-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");

        let mut cfg = Config::default();
        cfg.auth.api_base = Some("https://api.example.com".into());
        cfg.output.theme = "dark".into();
        cfg.save(&path).unwrap();

        let loaded = Config::load(Some(&path)).unwrap();
        assert_eq!(
            loaded.auth.api_base.as_deref(),
            Some("https://api.example.com")
        );
        assert_eq!(loaded.output.theme, "dark");

        std::fs::remove_dir_all(&dir).ok();
    }
}
