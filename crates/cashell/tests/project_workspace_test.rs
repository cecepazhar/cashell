//! Integration tests for CAShell Project Workspace data model & resource hierarchy.
//!
//! Validates:
//! - Project = workspace holding containers, repos, services, and DBs (multi-container workspace)
//! - Repo = code source inside project
//! - Container = Docker/Podman unit
//! - Service = running process / port
//! - DB = postgres / mysql / redis / sqlite
//! - Monorepo mapping: multiple repos, containers, services, DBs map under that project workspace
//! - CLI output & JSON serialization of the project hierarchy

use cashell::commands::project::{ProjectAction, ProjectCommand};
use cashell::config::Config;
use cashell::systat::{
    build_project_workspace, DockerContainerInfo, ProjectContainer, ProjectDatabase,
    ProjectRepo, ProjectService, TrackedProject,
};

#[test]
fn test_project_workspace_data_model_structure() {
    let ws = TrackedProject {
        path: "/home/user/Product/fathstore".to_string(),
        name: "fathstore".to_string(),
        category: "Product".to_string(),
        branch: "main".to_string(),
        is_dirty: false,
        repos: vec![
            ProjectRepo {
                name: "fathstore-core".to_string(),
                path: "apps/core".to_string(),
                branch: "main".to_string(),
                is_dirty: false,
            },
            ProjectRepo {
                name: "fathstore-pos".to_string(),
                path: "apps/pos".to_string(),
                branch: "feat-checkout".to_string(),
                is_dirty: true,
            },
        ],
        containers: vec![
            ProjectContainer {
                name: "fathstore-web".to_string(),
                image: "fathstore:latest".to_string(),
                state: "running".to_string(),
                status_detail: "Up 4 hours".to_string(),
                ports: "8082:8082".to_string(),
            },
            ProjectContainer {
                name: "fathstore-redis".to_string(),
                image: "redis:alpine".to_string(),
                state: "running".to_string(),
                status_detail: "Up 4 hours".to_string(),
                ports: "6379:6379".to_string(),
            },
        ],
        services: vec![
            ProjectService {
                name: "fathstore-api".to_string(),
                host: "127.0.0.1".to_string(),
                port: 8082,
                protocol: "http".to_string(),
                state: "up".to_string(),
            },
        ],
        databases: vec![
            ProjectDatabase {
                name: "fs_core".to_string(),
                engine: "postgres".to_string(),
                host: "127.0.0.1".to_string(),
                port: 5432,
                database: "fs_core".to_string(),
                state: "up".to_string(),
            },
            ProjectDatabase {
                name: "fathstore-redis".to_string(),
                engine: "redis".to_string(),
                host: "127.0.0.1".to_string(),
                port: 6379,
                database: "0".to_string(),
                state: "up".to_string(),
            },
        ],
    };

    // Verify summary line formatting
    assert_eq!(
        ws.summary_line(),
        "2 repo(s) · 2 container(s) · 1 service(s) · 2 db(s)"
    );

    // Verify JSON serialization round-trip
    let json_str = serde_json::to_string(&ws).expect("serialize workspace");
    let deserialized: TrackedProject = serde_json::from_str(&json_str).expect("deserialize workspace");
    assert_eq!(deserialized.name, "fathstore");
    assert_eq!(deserialized.repos.len(), 2);
    assert_eq!(deserialized.containers.len(), 2);
    assert_eq!(deserialized.services.len(), 1);
    assert_eq!(deserialized.databases.len(), 2);
    assert_eq!(deserialized.databases[0].engine, "postgres");
    assert_eq!(deserialized.databases[1].engine, "redis");
}

#[test]
fn test_monorepo_project_workspace_builder() {
    let tmp = std::env::temp_dir().join(format!("cashell_monorepo_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("crates/core")).unwrap();
    std::fs::create_dir_all(tmp.join("crates/web")).unwrap();
    std::fs::create_dir_all(tmp.join("apps/worker")).unwrap();

    // Create Cargo.toml / package.json in subpackages to emulate monorepo
    std::fs::write(tmp.join("Cargo.toml"), b"[workspace]\nmembers = [\"crates/*\", \"apps/*\"]").unwrap();
    std::fs::write(tmp.join("crates/core/Cargo.toml"), b"[package]\nname = \"core\"").unwrap();
    std::fs::write(tmp.join("crates/web/package.json"), b"{\"name\": \"web\"}").unwrap();
    std::fs::write(tmp.join("apps/worker/Cargo.toml"), b"[package]\nname = \"worker\"").unwrap();

    // Create docker-compose.yml defining web, worker, postgres, and redis
    let compose_content = r#"
version: '3.8'
services:
  web:
    image: mymono-web:latest
    ports:
      - "8080:8080"
  worker:
    image: mymono-worker:latest
    environment:
      - PORT=9000
  db:
    image: postgres:15-alpine
    ports:
      - "5432:5432"
  cache:
    image: redis:alpine
    ports:
      - "6379:6379"
"#;
    std::fs::write(tmp.join("docker-compose.yml"), compose_content).unwrap();

    // Create a local SQLite DB file
    std::fs::write(tmp.join("local_cache.db"), b"SQLITE FORMAT 3").unwrap();

    // Mock known container status
    let known_containers = vec![
        DockerContainerInfo {
            name: "mymono-web".to_string(),
            image: "mymono-web:latest".to_string(),
            state: "running".to_string(),
            status_detail: "Up 2 hours".to_string(),
            ports: "8080:8080".to_string(),
        },
    ];

    // Build workspace
    let ws = build_project_workspace("mymono", &tmp, "Product", &known_containers);

    assert_eq!(ws.name, "mymono");
    // Repos: crates/core, crates/web, apps/worker, plus root fallback
    assert!(ws.repos.iter().any(|r| r.name == "crates/core"));
    assert!(ws.repos.iter().any(|r| r.name == "crates/web"));
    assert!(ws.repos.iter().any(|r| r.name == "apps/worker"));
    assert!(ws.repos.len() >= 3, "expected at least 3 monorepo sub-repos, got {}", ws.repos.len());

    // Containers: web, worker, db, cache
    assert!(ws.containers.iter().any(|c| c.name.contains("web")));
    assert!(ws.containers.iter().any(|c| c.name.contains("worker") || c.name.contains("db")));

    // Databases: postgres, redis, and sqlite
    assert!(ws.databases.iter().any(|d| d.engine == "postgres"), "missing postgres db");
    assert!(ws.databases.iter().any(|d| d.engine == "redis"), "missing redis db");
    assert!(ws.databases.iter().any(|d| d.engine == "sqlite"), "missing sqlite local db");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_project_cli_command_json_output() {
    let cmd = ProjectCommand {
        action: Some(ProjectAction::List { active: false }),
        json: true,
    };
    let cfg = Config::default();
    let res = cashell::commands::project::run(&cmd, &cfg);
    assert!(res.is_ok());
    assert_eq!(res.unwrap(), 0);
}

#[test]
fn test_project_cli_info_command() {
    let cmd = ProjectCommand {
        action: Some(ProjectAction::Info { name: "cashell".to_string() }),
        json: true,
    };
    let cfg = Config::default();
    let res = cashell::commands::project::run(&cmd, &cfg);
    assert!(res.is_ok());
}
