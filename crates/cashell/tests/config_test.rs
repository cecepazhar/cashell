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
        cfg.output.theme = "tokyo-night".into();
        cfg.persona.user_name = "Dr. Cecep".into();
        cfg.database.retention_days = 14;
        cfg.save(&path).unwrap();

        let loaded = Config::load(Some(&path)).unwrap();
        assert_eq!(
            loaded.auth.api_base.as_deref(),
            Some("https://api.example.com")
        );
        assert_eq!(loaded.output.theme, "tokyo-night");
        assert_eq!(loaded.persona.user_name, "Dr. Cecep");
        assert_eq!(loaded.database.retention_days, 14);
        assert!(loaded.database.auto_prune);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn config_defaults_check() {
        let cfg = Config::default();
        assert_eq!(cfg.workspace.scan_dirs, vec!["~/Product", "~/Project"]);
        assert!(cfg
            .workspace
            .ignore_patterns
            .contains(&"target".to_string()));
        assert!(cfg
            .workspace
            .ignore_patterns
            .contains(&"node_modules".to_string()));
        assert_eq!(cfg.persona.user_name, "Prof. Cecep");
        assert_eq!(
            cfg.persona.assistant_role,
            "Kamu adalah Hana AI / Chief Operations Assistant Fathforce & CAShell. Terse, cekatan, dan patuh pada instruksi Prof. Cecep."
        );
        assert_eq!(cfg.database.retention_days, 7);
        assert!(cfg.database.auto_prune);
        assert!(cfg.output.theme == "ca-classic" || cfg.output.theme == "tokyo-night");
    }
}
