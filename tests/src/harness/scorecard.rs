use std::fmt::Display;

/// Pass/attempt counters for one peer implementation.
///
/// Metrics are labelled free-form ("mtools reading Hadris") and are reported
/// in first-use order. The headline percentage covers only the metrics named
/// through [`Scorecard::headline`], so auxiliary checks such as `fsck`
/// acceptance are reported without skewing the semantic accuracy score.
#[derive(Debug, Default)]
pub struct Scorecard {
    name: String,
    headline: Vec<String>,
    metrics: Vec<Metric>,
    pub command_failures: usize,
    pub details: Vec<String>,
}

#[derive(Debug, Default)]
struct Metric {
    label: String,
    passed: usize,
    attempted: usize,
}

impl Scorecard {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    pub fn headline(mut self, metrics: &[&str]) -> Self {
        self.headline = metrics.iter().map(|metric| metric.to_string()).collect();
        for metric in metrics {
            self.metric(metric);
        }
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    fn metric(&mut self, label: &str) -> &mut Metric {
        if let Some(index) = self.metrics.iter().position(|metric| metric.label == label) {
            return &mut self.metrics[index];
        }
        self.metrics.push(Metric {
            label: label.to_string(),
            ..Metric::default()
        });
        self.metrics.last_mut().unwrap()
    }

    pub fn attempt(&mut self, label: &str) {
        self.metric(label).attempted += 1;
    }

    pub fn pass(&mut self, label: &str) {
        self.metric(label).passed += 1;
    }

    /// Records one attempt and, on failure, one detail line prefixed with
    /// `context`. Returns whether the attempt passed.
    pub fn record(
        &mut self,
        label: &str,
        context: impl Display,
        result: Result<(), String>,
    ) -> bool {
        self.attempt(label);
        match result {
            Ok(()) => {
                self.pass(label);
                true
            }
            Err(error) => {
                self.details.push(format!("{context}: {error}"));
                false
            }
        }
    }

    pub fn command_failure(&mut self, detail: impl Into<String>) {
        self.command_failures += 1;
        self.details.push(detail.into());
    }

    pub fn passed(&self, label: &str) -> usize {
        self.metrics
            .iter()
            .find(|metric| metric.label == label)
            .map_or(0, |metric| metric.passed)
    }

    pub fn attempted(&self, label: &str) -> usize {
        self.metrics
            .iter()
            .find(|metric| metric.label == label)
            .map_or(0, |metric| metric.attempted)
    }

    pub fn all_passed(&self, label: &str) -> bool {
        self.passed(label) == self.attempted(label)
    }

    pub fn require_all(&self, metrics: &[(&str, usize)]) -> Result<(), String> {
        let incomplete = metrics
            .iter()
            .filter(|(label, expected)| {
                self.attempted(label) != *expected || self.passed(label) != *expected
            })
            .map(|(label, expected)| format!("{label} (expected {expected})"))
            .collect::<Vec<_>>();
        if incomplete.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "required metrics did not pass: {}\n{}",
                incomplete.join(", "),
                self.report()
            ))
        }
    }

    pub fn report(&self) -> String {
        let (passed, attempted) = self
            .metrics
            .iter()
            .filter(|metric| self.headline.contains(&metric.label))
            .fold((0, 0), |(passed, attempted), metric| {
                (passed + metric.passed, attempted + metric.attempted)
            });
        let percent = if attempted == 0 {
            0.0
        } else {
            passed as f64 * 100.0 / attempted as f64
        };
        let mut lines = vec![format!(
            "{} semantic accuracy: {passed}/{attempted} ({percent:.2}%)",
            self.name
        )];
        lines.extend(
            self.metrics
                .iter()
                .filter(|metric| metric.attempted > 0)
                .map(|metric| format!("{}: {}/{}", metric.label, metric.passed, metric.attempted)),
        );
        lines.push(format!(
            "external-tool command failures: {}",
            self.command_failures
        ));
        lines.extend(self.details.iter().cloned());
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::Scorecard;

    #[test]
    fn required_metrics_must_be_attempted() {
        let scorecard = Scorecard::new("peer");
        assert!(scorecard.require_all(&[("read", 1)]).is_err());
    }

    #[test]
    fn required_metrics_must_all_pass() {
        let mut scorecard = Scorecard::new("peer");
        scorecard.attempt("read");
        assert!(scorecard.require_all(&[("read", 1)]).is_err());
        scorecard.pass("read");
        assert!(scorecard.require_all(&[("read", 1)]).is_ok());
        assert!(scorecard.require_all(&[("read", 2)]).is_err());
    }
}
