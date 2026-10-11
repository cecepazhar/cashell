//! P2 layer self-checks: doctor report shape and AI config defaults (REQ-P2-01/02).

#[cfg(test)]
mod tests {
    use cashell::config::Config;

    #[test]
    fn ai_config_defaults_present() {
        let cfg = Config::default();
        assert!(!cfg.ai.fallback_chain.is_empty());
        assert_eq!(cfg.ai.fallback_chain[0], "omniroute");
        assert!(cfg.ai.providers.contains_key("omniroute"));
        assert!(cfg.ai.providers.contains_key("local"));
    }

    #[test]
    fn old_config_without_ai_loads() {
        // A config file predating the [ai] section must still deserialize.
        let legacy = "[auth]\napi_base = \"https://api.example.com\"\n";
        let cfg: Config = toml::from_str(legacy).expect("legacy config must load");
        assert_eq!(cfg.ai.default_provider, "omniroute");
    }

    #[test]
    fn ai_config_roundtrip() {
        let dir = std::env::temp_dir().join("cashell-test-p2");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let cfg = Config::default();
        cfg.save(&path).unwrap();
        let loaded = Config::load(Some(&path)).unwrap();
        assert_eq!(loaded.ai.fallback_chain, cfg.ai.fallback_chain);
        std::fs::remove_dir_all(&dir).ok();
    }
}
