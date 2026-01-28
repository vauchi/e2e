//! YAML Scenario Loader
//!
//! Loads and validates YAML scenario files from the filesystem.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{E2eError, E2eResult};

use super::schema::{Platform, Scenario};

/// Loader for YAML scenarios.
pub struct ScenarioLoader {
    /// Base directory for scenarios.
    base_dir: PathBuf,

    /// Cached scenarios.
    cache: HashMap<String, Scenario>,
}

impl ScenarioLoader {
    /// Create a new loader with the given base directory.
    pub fn new(base_dir: impl AsRef<Path>) -> Self {
        Self {
            base_dir: base_dir.as_ref().to_path_buf(),
            cache: HashMap::new(),
        }
    }

    /// Create a loader for the default scenarios directory.
    pub fn default() -> Self {
        // Try to find the scenarios directory relative to the crate
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
        let scenarios_dir = PathBuf::from(manifest_dir).join("scenarios");

        if scenarios_dir.exists() {
            Self::new(scenarios_dir)
        } else {
            // Fall back to e2e/scenarios from workspace root
            Self::new("e2e/scenarios")
        }
    }

    /// Load a scenario by name.
    ///
    /// The name can be:
    /// - A simple name (e.g., "cli-exchange") - looks in all subdirectories
    /// - A path-like name (e.g., "smoke/cli-exchange") - looks in specific directory
    pub fn load(&mut self, name: &str) -> E2eResult<&Scenario> {
        // Check cache first
        if self.cache.contains_key(name) {
            return Ok(self.cache.get(name).unwrap());
        }

        // Find the scenario file
        let path = self.find_scenario_file(name)?;

        // Load and parse
        let content = std::fs::read_to_string(&path).map_err(|e| {
            E2eError::ScenarioLoad(format!("Failed to read {}: {}", path.display(), e))
        })?;

        let scenario: Scenario = serde_yaml::from_str(&content).map_err(|e| {
            E2eError::ScenarioLoad(format!("Failed to parse {}: {}", path.display(), e))
        })?;

        // Validate
        self.validate(&scenario)?;

        // Cache and return
        self.cache.insert(name.to_string(), scenario);
        Ok(self.cache.get(name).unwrap())
    }

    /// Load all scenarios from a directory.
    pub fn load_dir(&mut self, subdir: &str) -> E2eResult<Vec<&Scenario>> {
        let dir = self.base_dir.join(subdir);
        if !dir.exists() {
            return Err(E2eError::ScenarioLoad(format!(
                "Directory not found: {}",
                dir.display()
            )));
        }

        let mut names = Vec::new();

        for entry in std::fs::read_dir(&dir).map_err(|e| {
            E2eError::ScenarioLoad(format!("Failed to read {}: {}", dir.display(), e))
        })? {
            let entry = entry
                .map_err(|e| E2eError::ScenarioLoad(format!("Failed to read entry: {}", e)))?;
            let path = entry.path();

            if path
                .extension()
                .map_or(false, |ext| ext == "yaml" || ext == "yml")
            {
                let name = format!("{}/{}", subdir, path.file_stem().unwrap().to_string_lossy());
                names.push(name);
            }
        }

        // Load all scenarios
        for name in &names {
            self.load(name)?;
        }

        // Return references
        Ok(names.iter().filter_map(|n| self.cache.get(n)).collect())
    }

    /// Load all scenarios matching given tags.
    pub fn load_by_tags(&mut self, tags: &[&str]) -> E2eResult<Vec<&Scenario>> {
        self.load_all()?;

        Ok(self
            .cache
            .values()
            .filter(|s| tags.iter().any(|t| s.tags.contains(&t.to_string())))
            .collect())
    }

    /// Load all scenarios.
    pub fn load_all(&mut self) -> E2eResult<Vec<&Scenario>> {
        let subdirs = ["smoke", "integration", "edge"];

        for subdir in subdirs {
            let dir = self.base_dir.join(subdir);
            if dir.exists() {
                let _ = self.load_dir(subdir);
            }
        }

        Ok(self.cache.values().collect())
    }

    /// List available scenarios.
    pub fn list(&self) -> E2eResult<Vec<ScenarioInfo>> {
        let mut infos = Vec::new();

        for subdir in ["smoke", "integration", "edge"] {
            let dir = self.base_dir.join(subdir);
            if !dir.exists() {
                continue;
            }

            for entry in std::fs::read_dir(&dir).map_err(|e| {
                E2eError::ScenarioLoad(format!("Failed to read {}: {}", dir.display(), e))
            })? {
                let entry = entry
                    .map_err(|e| E2eError::ScenarioLoad(format!("Failed to read entry: {}", e)))?;
                let path = entry.path();

                if path
                    .extension()
                    .map_or(false, |ext| ext == "yaml" || ext == "yml")
                {
                    let content = std::fs::read_to_string(&path).ok();
                    let info = if let Some(content) = content {
                        if let Ok(scenario) = serde_yaml::from_str::<Scenario>(&content) {
                            ScenarioInfo {
                                name: scenario.name,
                                path: format!(
                                    "{}/{}",
                                    subdir,
                                    path.file_stem().unwrap().to_string_lossy()
                                ),
                                tags: scenario.tags,
                                feature: scenario.feature,
                                manual_only: scenario.manual_only,
                            }
                        } else {
                            continue;
                        }
                    } else {
                        continue;
                    };

                    infos.push(info);
                }
            }
        }

        Ok(infos)
    }

    /// Find the scenario file for a given name.
    fn find_scenario_file(&self, name: &str) -> E2eResult<PathBuf> {
        // Check if it's a direct path
        if name.contains('/') {
            let path = self.base_dir.join(format!("{}.yaml", name));
            if path.exists() {
                return Ok(path);
            }
            let path = self.base_dir.join(format!("{}.yml", name));
            if path.exists() {
                return Ok(path);
            }
            return Err(E2eError::ScenarioLoad(format!(
                "Scenario not found: {}",
                name
            )));
        }

        // Search in subdirectories
        for subdir in ["smoke", "integration", "edge", "."] {
            let path = self.base_dir.join(subdir).join(format!("{}.yaml", name));
            if path.exists() {
                return Ok(path);
            }
            let path = self.base_dir.join(subdir).join(format!("{}.yml", name));
            if path.exists() {
                return Ok(path);
            }
        }

        Err(E2eError::ScenarioLoad(format!(
            "Scenario not found: {}",
            name
        )))
    }

    /// Validate a scenario.
    fn validate(&self, scenario: &Scenario) -> E2eResult<()> {
        // Check that scenario has a name
        if scenario.name.is_empty() {
            return Err(E2eError::ScenarioLoad(
                "Scenario must have a name".to_string(),
            ));
        }

        // Check that scenario has steps
        if scenario.steps.is_empty() {
            return Err(E2eError::ScenarioLoad(format!(
                "Scenario '{}' has no steps",
                scenario.name
            )));
        }

        // Validate participants exist in steps
        // (This is a basic validation - executor does more thorough checks)

        Ok(())
    }

    /// Filter scenarios by platform capability.
    pub fn filter_by_platform<'a>(
        scenarios: &'a [&'a Scenario],
        platform: Platform,
    ) -> Vec<&'a Scenario> {
        scenarios
            .iter()
            .filter(|s| {
                if let Some(platforms) = &s.platforms {
                    platforms.contains(&platform) || platforms.contains(&Platform::Any)
                } else {
                    true // No platform restriction
                }
            })
            .copied()
            .collect()
    }
}

/// Information about a scenario (for listing).
#[derive(Debug, Clone)]
pub struct ScenarioInfo {
    /// Display name.
    pub name: String,

    /// Path to load the scenario.
    pub path: String,

    /// Tags.
    pub tags: Vec<String>,

    /// Gherkin feature reference.
    pub feature: Option<String>,

    /// Whether this is manual-only.
    pub manual_only: bool,
}

impl std::fmt::Display for ScenarioInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)?;
        if !self.tags.is_empty() {
            write!(f, " [{}]", self.tags.join(", "))?;
        }
        if self.manual_only {
            write!(f, " (manual)")?;
        }
        if let Some(feature) = &self.feature {
            write!(f, " -> {}", feature)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_test_scenario(dir: &Path, subdir: &str, name: &str, content: &str) {
        let subdir_path = dir.join(subdir);
        std::fs::create_dir_all(&subdir_path).unwrap();
        let file_path = subdir_path.join(format!("{}.yaml", name));
        let mut file = std::fs::File::create(file_path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
    }

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
