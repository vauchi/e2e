//! Test Reporting Module
//!
//! Generates JUnit XML reports for CI integration and human-readable summaries.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use crate::yaml::ScenarioResult;

/// JUnit XML report generator.
pub struct JunitReport {
    /// Test suite name.
    suite_name: String,

    /// Individual test results.
    test_cases: Vec<TestCase>,

    /// Total execution time.
    total_time: Duration,
}

/// A single test case in the report.
struct TestCase {
    name: String,
    class_name: String,
    time: Duration,
    status: TestStatus,
    message: Option<String>,
}

/// Test case status.
enum TestStatus {
    Passed,
    Failed,
    Skipped,
    Error,
}

impl JunitReport {
    /// Create a new JUnit report.
    pub fn new(suite_name: impl Into<String>) -> Self {
        Self {
            suite_name: suite_name.into(),
            test_cases: Vec::new(),
            total_time: Duration::ZERO,
        }
    }

    /// Add a scenario result to the report.
    pub fn add_scenario_result(&mut self, result: &ScenarioResult) {
        self.total_time += result.duration;

        let status = if result.passed {
            TestStatus::Passed
        } else {
            TestStatus::Failed
        };

        self.test_cases.push(TestCase {
            name: result.name.clone(),
            class_name: "e2e".to_string(),
            time: result.duration,
            status,
            message: result.error.clone(),
        });
    }

    /// Add a passed test case.
    pub fn add_passed(&mut self, name: &str, class_name: &str, time: Duration) {
        self.total_time += time;
        self.test_cases.push(TestCase {
            name: name.to_string(),
            class_name: class_name.to_string(),
            time,
            status: TestStatus::Passed,
            message: None,
        });
    }

    /// Add a failed test case.
    pub fn add_failed(&mut self, name: &str, class_name: &str, time: Duration, message: &str) {
        self.total_time += time;
        self.test_cases.push(TestCase {
            name: name.to_string(),
            class_name: class_name.to_string(),
            time,
            status: TestStatus::Failed,
            message: Some(message.to_string()),
        });
    }

    /// Add a skipped test case.
    pub fn add_skipped(&mut self, name: &str, class_name: &str, message: Option<&str>) {
        self.test_cases.push(TestCase {
            name: name.to_string(),
            class_name: class_name.to_string(),
            time: Duration::ZERO,
            status: TestStatus::Skipped,
            message: message.map(|s| s.to_string()),
        });
    }

    /// Add an error test case.
    pub fn add_error(&mut self, name: &str, class_name: &str, time: Duration, message: &str) {
        self.total_time += time;
        self.test_cases.push(TestCase {
            name: name.to_string(),
            class_name: class_name.to_string(),
            time,
            status: TestStatus::Error,
            message: Some(message.to_string()),
        });
    }

    /// Get counts of passed, failed, skipped, and error tests.
    pub fn counts(&self) -> (usize, usize, usize, usize) {
        let mut passed = 0;
        let mut failed = 0;
        let mut skipped = 0;
        let mut errors = 0;

        for tc in &self.test_cases {
            match tc.status {
                TestStatus::Passed => passed += 1,
                TestStatus::Failed => failed += 1,
                TestStatus::Skipped => skipped += 1,
                TestStatus::Error => errors += 1,
            }
        }

        (passed, failed, skipped, errors)
    }

    /// Generate JUnit XML format.
    pub fn to_xml(&self) -> String {
        let (_passed, failed, skipped, errors) = self.counts();
        let total = self.test_cases.len();

        let mut xml = String::new();
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        xml.push_str(&format!(
            "<testsuite name=\"{}\" tests=\"{}\" failures=\"{}\" errors=\"{}\" skipped=\"{}\" time=\"{:.3}\">\n",
            escape_xml(&self.suite_name),
            total,
            failed,
            errors,
            skipped,
            self.total_time.as_secs_f64()
        ));

        for tc in &self.test_cases {
            xml.push_str(&format!(
                "  <testcase name=\"{}\" classname=\"{}\" time=\"{:.3}\"",
                escape_xml(&tc.name),
                escape_xml(&tc.class_name),
                tc.time.as_secs_f64()
            ));

            match &tc.status {
                TestStatus::Passed => {
                    xml.push_str("/>\n");
                }
                TestStatus::Failed => {
                    xml.push_str(">\n");
                    if let Some(msg) = &tc.message {
                        xml.push_str(&format!(
                            "    <failure message=\"{}\">{}</failure>\n",
                            escape_xml(msg),
                            escape_xml(msg)
                        ));
                    } else {
                        xml.push_str("    <failure/>\n");
                    }
                    xml.push_str("  </testcase>\n");
                }
                TestStatus::Skipped => {
                    xml.push_str(">\n");
                    if let Some(msg) = &tc.message {
                        xml.push_str(&format!("    <skipped message=\"{}\"/>\n", escape_xml(msg)));
                    } else {
                        xml.push_str("    <skipped/>\n");
                    }
                    xml.push_str("  </testcase>\n");
                }
                TestStatus::Error => {
                    xml.push_str(">\n");
                    if let Some(msg) = &tc.message {
                        xml.push_str(&format!(
                            "    <error message=\"{}\">{}</error>\n",
                            escape_xml(msg),
                            escape_xml(msg)
                        ));
                    } else {
                        xml.push_str("    <error/>\n");
                    }
                    xml.push_str("  </testcase>\n");
                }
            }
        }

        xml.push_str("</testsuite>\n");
        xml
    }

    /// Write JUnit XML to a file.
    pub fn write_to_file(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let mut file = std::fs::File::create(path)?;
        file.write_all(self.to_xml().as_bytes())?;
        Ok(())
    }

    /// Generate a human-readable summary.
    pub fn summary(&self) -> String {
        let (passed, failed, skipped, errors) = self.counts();
        let total = self.test_cases.len();

        let mut summary = String::new();
        summary.push_str(&format!("\nE2E Test Results: {}\n", self.suite_name));
        summary.push_str(&"=".repeat(50));
        summary.push('\n');
        summary.push_str(&format!(
            "Total: {} | Passed: {} | Failed: {} | Skipped: {} | Errors: {}\n",
            total, passed, failed, skipped, errors
        ));
        summary.push_str(&format!("Duration: {:.2}s\n", self.total_time.as_secs_f64()));
        summary.push('\n');

        // List failed and error tests
        let failures: Vec<_> = self
            .test_cases
            .iter()
            .filter(|tc| matches!(tc.status, TestStatus::Failed | TestStatus::Error))
            .collect();

        if !failures.is_empty() {
            summary.push_str("Failed Tests:\n");
            for tc in failures {
                summary.push_str(&format!("  - {} ", tc.name));
                if let Some(msg) = &tc.message {
                    // Truncate long messages
                    let truncated = if msg.len() > 80 {
                        format!("{}...", &msg[..77])
                    } else {
                        msg.clone()
                    };
                    summary.push_str(&format!("({})", truncated));
                }
                summary.push('\n');
            }
        }

        summary
    }

    /// Print summary to stdout.
    pub fn print_summary(&self) {
        println!("{}", self.summary());
    }
}

/// Escape special XML characters.
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Summary statistics for a test run.
#[derive(Debug, Clone, Default)]
pub struct TestSummary {
    /// Total number of tests.
    pub total: usize,
    /// Number of passed tests.
    pub passed: usize,
    /// Number of failed tests.
    pub failed: usize,
    /// Number of skipped tests.
    pub skipped: usize,
    /// Number of tests with errors.
    pub errors: usize,
    /// Total execution time.
    pub duration: Duration,
    /// Individual test results (name -> passed).
    pub results: HashMap<String, bool>,
}

impl TestSummary {
    /// Create a new test summary.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a test result.
    pub fn record(&mut self, name: &str, passed: bool, duration: Duration) {
        self.total += 1;
        self.duration += duration;

        if passed {
            self.passed += 1;
        } else {
            self.failed += 1;
        }

        self.results.insert(name.to_string(), passed);
    }

    /// Check if all tests passed.
    pub fn all_passed(&self) -> bool {
        self.failed == 0 && self.errors == 0
    }

    /// Get the pass rate as a percentage.
    pub fn pass_rate(&self) -> f64 {
        if self.total == 0 {
            100.0
        } else {
            (self.passed as f64 / self.total as f64) * 100.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_junit_report_xml() {
        let mut report = JunitReport::new("e2e-smoke");
        report.add_passed("smoke_cli_exchange", "e2e.smoke", Duration::from_millis(1500));
        report.add_failed(
            "smoke_sync",
            "e2e.smoke",
            Duration::from_millis(500),
            "Assertion failed: expected 1 contact",
        );

        let xml = report.to_xml();
        assert!(xml.contains("<?xml version=\"1.0\""));
        assert!(xml.contains("testsuite name=\"e2e-smoke\""));
        assert!(xml.contains("tests=\"2\""));
        assert!(xml.contains("failures=\"1\""));
        assert!(xml.contains("smoke_cli_exchange"));
        assert!(xml.contains("<failure"));
    }

    #[test]
    fn test_escape_xml() {
        assert_eq!(escape_xml("<test>"), "&lt;test&gt;");
        assert_eq!(escape_xml("foo & bar"), "foo &amp; bar");
        assert_eq!(escape_xml("\"quoted\""), "&quot;quoted&quot;");
    }

    #[test]
    fn test_summary() {
        let mut summary = TestSummary::new();
        summary.record("test1", true, Duration::from_secs(1));
        summary.record("test2", true, Duration::from_secs(2));
        summary.record("test3", false, Duration::from_secs(1));

        assert_eq!(summary.total, 3);
        assert_eq!(summary.passed, 2);
        assert_eq!(summary.failed, 1);
        assert!(!summary.all_passed());
        assert!((summary.pass_rate() - 66.67).abs() < 0.1);
    }
}
