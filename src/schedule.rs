//! The order in which the picked legs run: one after another, or `--parallel N` at
//! the same time, with each leg after the jobs it `needs` (ADR-0011).

use crate::plan::Selection;
use crate::run::{run_leg, skipped, LegReport, Options};
use crate::workflow::{Leg, Workflow};
use serde_json::{json, Map, Value};
use std::sync::mpsc;

pub fn run_all(
    legs: &[&Leg],
    workflow: &Workflow,
    base: &Map<String, Value>,
    sel: &Selection,
    opts: &Options,
) -> Vec<LegReport> {
    if opts.parallel > 1 {
        return run_parallel(legs, workflow, base, sel, opts);
    }
    let mut needs = Map::new();
    let mut reports = Vec::new();
    let mut failed = false;
    for leg in order(legs) {
        if failed && !opts.keep_going {
            reports.push(skipped(leg, FAIL_FAST.into()));
            continue;
        }
        let (report, outputs) = run_leg(leg, workflow, base, &needs, sel, opts);
        failed |= report.result == "failure";
        needs.insert(
            leg.job.id.clone(),
            json!({ "result": report.result, "outputs": outputs }),
        );
        reports.push(report);
    }
    reports
}

/// Picked legs with every picked dependency before its dependents.
fn order<'a, 'w>(legs: &[&'a Leg<'w>]) -> Vec<&'a Leg<'w>> {
    let mut pending: Vec<&Leg> = legs.to_vec();
    let mut out: Vec<&Leg> = Vec::new();
    while !pending.is_empty() {
        let ready = pending.iter().position(|leg| {
            leg.job
                .needs
                .list()
                .iter()
                .all(|n| !pending.iter().any(|p| &p.job.id == n))
        });
        out.push(pending.remove(ready.unwrap_or(0)));
    }
    out
}

const FAIL_FAST: &str = "an earlier leg failed (use --keep-going to continue)";

/// Up to `opts.parallel` legs at the same time. A leg starts when every picked leg of
/// the jobs it needs has finished. The reports keep the dependency order.
fn run_parallel(
    legs: &[&Leg],
    workflow: &Workflow,
    base: &Map<String, Value>,
    sel: &Selection,
    opts: &Options,
) -> Vec<LegReport> {
    let ordered = order(legs);
    let n = ordered.len();
    let mut reports: Vec<Option<LegReport>> = (0..n).map(|_| None).collect();
    let mut needs = Map::new();
    let mut done = vec![false; n];
    let mut started = vec![false; n];
    let mut running = 0;
    let mut failed = false;
    std::thread::scope(|scope| {
        let (tx, rx) = mpsc::channel::<(usize, LegReport, Value)>();
        loop {
            for i in 0..n {
                if started[i] || running >= opts.parallel {
                    continue;
                }
                let leg = ordered[i];
                let waiting = leg
                    .job
                    .needs
                    .list()
                    .iter()
                    .any(|need| (0..n).any(|j| !done[j] && &ordered[j].job.id == need));
                if waiting {
                    continue;
                }
                started[i] = true;
                if failed && !opts.keep_going {
                    reports[i] = Some(skipped(leg, FAIL_FAST.into()));
                    done[i] = true;
                    continue;
                }
                running += 1;
                let (tx, snapshot) = (tx.clone(), needs.clone());
                scope.spawn(move || {
                    let (report, outputs) = run_leg(leg, workflow, base, &snapshot, sel, opts);
                    let _ = tx.send((i, report, outputs));
                });
            }
            if running == 0 {
                for i in (0..n).filter(|i| !started[*i]) {
                    reports[i] = Some(skipped(ordered[i], "its needs never finished".into()));
                }
                break;
            }
            let Ok((i, report, outputs)) = rx.recv() else {
                break;
            };
            running -= 1;
            done[i] = true;
            failed |= report.result == "failure";
            needs.insert(
                ordered[i].job.id.clone(),
                json!({ "result": report.result, "outputs": outputs }),
            );
            reports[i] = Some(report);
        }
    });
    reports.into_iter().flatten().collect()
}
