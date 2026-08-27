//! Scoring drivers that measure one peer against the oracle and Hadris.

use std::fs;
use std::path::Path;

use super::adapter::{IsoConsumer, IsoProducer};
use super::model::{IsoState, compare_entries, compare_state};
use super::{hadris, spec};
use crate::harness::Scorecard;

pub struct Labels {
    pub reads: String,
    pub writes: String,
    pub hadris_reads: String,
}

pub fn labels(peer: &str) -> Labels {
    Labels {
        reads: format!("{peer} reading Hadris"),
        writes: format!("{peer} writing spec-valid images"),
        hadris_reads: format!("Hadris reading {peer} images"),
    }
}

pub fn scorecard(peer: &str) -> Scorecard {
    let labels = labels(peer);
    Scorecard::new(peer).headline(&[&labels.reads, &labels.writes])
}

/// Has the producer author `expected`, then checks the image with the raw
/// oracle and the Hadris reader. A write passes only when both agree.
pub fn producer(
    scorecard: &mut Scorecard,
    scenario: &str,
    expected: &IsoState,
    producer: &dyn IsoProducer,
    workspace: &Path,
) -> Result<(), String> {
    let name = producer.name();
    let labels = labels(&name);
    let image = workspace.join(format!("{}.iso", slug(&name)));
    scorecard.attempt(&labels.writes);
    if let Err(error) = producer.produce(expected, workspace, &image) {
        scorecard.command_failure(format!(
            "{scenario} {name} producer command failed: {error}"
        ));
        return Ok(());
    }
    let bytes = fs::read(&image).map_err(|error| error.to_string())?;
    let oracle = spec::snapshot(&bytes)
        .and_then(|state| compare_state("raw ECMA-119 oracle", expected, &state));
    if let Err(error) = &oracle {
        scorecard
            .details
            .push(format!("{scenario} {name} spec mismatch: {error}"));
    }
    let hadris = scorecard.record(
        &labels.hadris_reads,
        format!("{scenario} Hadris read mismatch"),
        hadris::snapshot(bytes).and_then(|state| compare_state("Hadris reader", expected, &state)),
    );
    if oracle.is_ok() && hadris {
        scorecard.pass(&labels.writes);
    }
    Ok(())
}

/// Writes `expected` with Hadris, confirms the image with the oracle, then
/// checks that the consumer reads the same tree.
pub fn consumer(
    scorecard: &mut Scorecard,
    scenario: &str,
    expected: &IsoState,
    consumer: &dyn IsoConsumer,
    workspace: &Path,
) -> Result<(), String> {
    let name = consumer.name();
    let labels = labels(&name);
    let image = workspace.join("hadris.iso");
    let bytes = hadris::write(expected)?;
    compare_state(
        &format!("Hadris image before {name} measurement"),
        expected,
        &spec::snapshot(&bytes)?,
    )?;
    fs::write(&image, bytes).map_err(|error| error.to_string())?;
    scorecard.record(
        &labels.reads,
        format!("{scenario} {name} read mismatch"),
        consumer
            .snapshot(&image, workspace)
            .and_then(|state| compare_entries(&format!("{name} reader"), expected, &state)),
    );
    Ok(())
}

fn slug(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}
