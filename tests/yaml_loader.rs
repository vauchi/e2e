// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

mod scenario_loader {
    use std::path::Path;

    use std::io::Write;
    use tempfile::TempDir;
    use vauchi_e2e_tests::yaml::ScenarioLoader;

    fn create_test_scenario(dir: &Path, subdir: &str, name: &str, content: &str) {
        let subdir_path = dir.join(subdir);
        std::fs::create_dir_all(&subdir_path).unwrap();
        let file_path = subdir_path.join(format!("{}.yaml", name));
        let mut file = std::fs::File::create(file_path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
    }

    // @internal
    #[test]
    fn test_load_scenario() {
        let temp_dir = TempDir::new().unwrap();
        let content = r#"
name: "Test Scenario"
tags: ["smoke"]
participants:
  alice: { devices: 1 }
steps:
  - type: action
    action: create_identity
    actor: alice
    params:
      name: "Alice"
"#;
        create_test_scenario(temp_dir.path(), "smoke", "test", content);

        let mut loader = ScenarioLoader::new(temp_dir.path());
        let scenario = loader.load("test").unwrap();

        assert_eq!(scenario.name, "Test Scenario");
        assert_eq!(scenario.tags, vec!["smoke"]);
    }

    // @internal
    #[test]
    fn test_load_by_path() {
        let temp_dir = TempDir::new().unwrap();
        let content = r#"
name: "Integration Test"
tags: ["integration"]
participants:
  bob: { devices: 2 }
steps:
  - type: action
    action: create_identity
    actor: bob
    params:
      name: "Bob"
"#;
        create_test_scenario(temp_dir.path(), "integration", "multi-device", content);

        let mut loader = ScenarioLoader::new(temp_dir.path());
        let scenario = loader.load("integration/multi-device").unwrap();

        assert_eq!(scenario.name, "Integration Test");
    }

    // @internal
    #[test]
    fn test_list_scenarios() {
        let temp_dir = TempDir::new().unwrap();

        create_test_scenario(
            temp_dir.path(),
            "smoke",
            "exchange",
            r#"
name: "Exchange Test"
tags: ["smoke", "exchange"]
steps:
  - type: action
    action: create_identity
    actor: alice
"#,
        );

        create_test_scenario(
            temp_dir.path(),
            "integration",
            "sync",
            r#"
name: "Sync Test"
tags: ["integration", "sync"]
feature: "sync_updates.feature"
steps:
  - type: action
    action: sync
    actor: alice
"#,
        );

        let loader = ScenarioLoader::new(temp_dir.path());
        let infos = loader.list().unwrap();

        assert_eq!(infos.len(), 2);
    }
}
