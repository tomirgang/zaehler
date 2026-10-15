//! Prüft die hexagonalen Abhängigkeitsregeln (SPEC_ARCH_LAYERS, ADR_022)
//! anhand von `cargo metadata`.

use std::{collections::BTreeMap, process::Command};

use serde_json::Value;

const ADAPTERS: [&str; 6] = [
    "zaehler-web",
    "zaehler-db",
    "zaehler-ai",
    "zaehler-notify",
    "zaehler-storage",
    "zaehler-media",
];
const FRAMEWORKS: [&str; 8] = [
    "axum",
    "sea-orm",
    "reqwest",
    "askama",
    "tower-sessions",
    "object_store",
    "image",
    "tokio",
];

/// Workspace-Crate → direkte (normale) Abhängigkeiten.
fn dependencies() -> BTreeMap<String, Vec<String>> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--offline",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo metadata");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let meta: Value = serde_json::from_slice(&out.stdout).expect("JSON");
    meta["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .map(|p| {
            let deps = p["dependencies"]
                .as_array()
                .expect("dependencies")
                .iter()
                .filter(|d| d["kind"].is_null()) // nur normale Abhängigkeiten, keine dev/build
                .map(|d| d["name"].as_str().unwrap_or_default().to_owned())
                .collect();
            (p["name"].as_str().unwrap_or_default().to_owned(), deps)
        })
        .collect()
}

// @test SPEC_ARCH_LAYERS
#[test]
fn hexagonal_dependency_rules_hold() {
    let deps = dependencies();
    let workspace: Vec<&String> = deps.keys().collect();
    let internal = |crate_name: &str| -> Vec<String> {
        deps[crate_name]
            .iter()
            .filter(|d| workspace.contains(d))
            .cloned()
            .collect()
    };

    assert!(
        internal("zaehler-domain").is_empty(),
        "domain hängt von keinem Workspace-Crate ab"
    );
    assert_eq!(
        internal("zaehler-app"),
        ["zaehler-domain"],
        "app hängt nur von domain ab"
    );
    for framework in FRAMEWORKS {
        assert!(
            !deps["zaehler-app"].iter().any(|d| d == framework)
                && !deps["zaehler-domain"].iter().any(|d| d == framework),
            "Kern ohne {framework}"
        );
    }
    for adapter in ADAPTERS.iter().filter(|a| deps.contains_key(**a)) {
        for dep in internal(adapter) {
            assert!(
                dep == "zaehler-app" || dep == "zaehler-domain",
                "{adapter} darf nur von zaehler-app/zaehler-domain abhängen, nicht von {dep}"
            );
        }
    }
    for (name, list) in &deps {
        if name != "zaehler-server" {
            assert!(
                !list.iter().any(|d| d == "zaehler-server"),
                "{name} hängt von der Composition Root ab"
            );
        }
    }
}
