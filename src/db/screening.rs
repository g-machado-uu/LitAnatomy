//! Systematic-review screening: a collection's inclusion and exclusion
//! criteria, and each paper's decision in that collection.

use std::collections::HashMap;

use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, Row, params};

use super::Library;
use crate::model::{Criterion, CriterionKind, Decision, Screening, now_string};

fn screening_from_row(r: &Row, first: usize) -> rusqlite::Result<Screening> {
    let criteria: String = r.get(first + 1)?;
    Ok(Screening {
        decision: Decision::from_i64(r.get(first)?),
        criteria: serde_json::from_str(&criteria).unwrap_or_default(),
        note: r.get(first + 2)?,
        decided_at: r.get(first + 3)?,
    })
}

/// How far a review has got: papers per decision, and exclusions per criterion.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScreeningSummary {
    pub counts: HashMap<Decision, usize>,
    /// (criterion, excluded papers that cite it), in the criteria's order.
    pub excluded_by: Vec<(i64, usize)>,
    /// Excluded papers with no criterion ticked.
    pub excluded_without_reason: usize,
}

impl ScreeningSummary {
    pub fn count(&self, d: Decision) -> usize {
        self.counts.get(&d).copied().unwrap_or(0)
    }

    pub fn total(&self) -> usize {
        self.counts.values().sum()
    }
}

impl Library {
    /// A review's criteria: inclusion first, then exclusion, each in the order added.
    pub fn criteria(&self, collection: i64) -> Result<Vec<Criterion>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, collection_id, kind, label, position FROM criteria WHERE collection_id = ?1
             ORDER BY kind = 'exclude', position, id",
        )?;
        let rows = stmt.query_map([collection], |r| {
            Ok(Criterion {
                id: r.get(0)?,
                collection: r.get(1)?,
                kind: CriterionKind::parse(&r.get::<_, String>(2)?),
                label: r.get(3)?,
                position: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn add_criterion(&mut self, collection: i64, kind: CriterionKind, label: &str) -> Result<i64> {
        if label.trim().is_empty() {
            bail!("Describe the criterion first.");
        }
        let position: i64 = self.conn.query_row(
            "SELECT coalesce(max(position), -1) + 1 FROM criteria WHERE collection_id = ?1 AND kind = ?2",
            params![collection, kind.as_str()],
            |r| r.get(0),
        )?;
        self.conn.execute(
            "INSERT INTO criteria (collection_id, kind, label, position) VALUES (?1, ?2, ?3, ?4)",
            params![collection, kind.as_str(), label.trim(), position],
        )?;
        self.touch();
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_criterion(&mut self, id: i64, label: &str) -> Result<()> {
        if label.trim().is_empty() {
            bail!("A criterion can't be empty. Delete it instead.");
        }
        self.conn.execute("UPDATE criteria SET label = ?2 WHERE id = ?1", params![id, label.trim()])?;
        self.touch();
        Ok(())
    }

    /// Deletes a criterion, and un-ticks it on the papers that cited it.
    pub fn delete_criterion(&mut self, id: i64) -> Result<()> {
        let Some(collection) = self
            .conn
            .query_row("SELECT collection_id FROM criteria WHERE id = ?1", [id], |r| r.get::<_, i64>(0))
            .optional()?
        else {
            return Ok(());
        };
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "SELECT paper_id, decision_criteria FROM collection_papers
                 WHERE collection_id = ?1 AND decision_criteria <> '[]'",
            )?;
            let rows: Vec<(i64, String)> =
                stmt.query_map([collection], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            for (paper, json) in rows {
                let mut ids: Vec<i64> = serde_json::from_str(&json).unwrap_or_default();
                if ids.contains(&id) {
                    ids.retain(|c| *c != id);
                    tx.execute(
                        "UPDATE collection_papers SET decision_criteria = ?3 WHERE collection_id = ?1 AND paper_id = ?2",
                        params![collection, paper, serde_json::to_string(&ids)?],
                    )?;
                }
            }
        }
        tx.execute("DELETE FROM criteria WHERE id = ?1", [id])?;
        tx.commit()?;
        self.touch();
        Ok(())
    }

    /// A paper's screening in a collection, or None if it isn't in it.
    pub fn screening(&self, collection: i64, paper: i64) -> Result<Option<Screening>> {
        Ok(self
            .conn
            .query_row(
                "SELECT decision, decision_criteria, decision_note, decided_at FROM collection_papers
                 WHERE collection_id = ?1 AND paper_id = ?2",
                params![collection, paper],
                |r| screening_from_row(r, 0),
            )
            .optional()?)
    }

    /// Every paper's screening in a collection (papers in the Trash left out).
    pub fn screenings(&self, collection: i64) -> Result<HashMap<i64, Screening>> {
        let mut stmt = self.conn.prepare(
            "SELECT cp.paper_id, cp.decision, cp.decision_criteria, cp.decision_note, cp.decided_at
             FROM collection_papers cp JOIN papers p ON p.id = cp.paper_id
             WHERE cp.collection_id = ?1 AND p.deleted_at IS NULL",
        )?;
        let rows = stmt.query_map([collection], |r| Ok((r.get::<_, i64>(0)?, screening_from_row(r, 1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_screening(&mut self, collection: i64, paper: i64, s: &Screening) -> Result<()> {
        let now = now_string();
        let decided_at = (s.decision != Decision::Undecided).then(|| s.decided_at.clone().unwrap_or_else(|| now.clone()));
        let changed = self.conn.execute(
            "UPDATE collection_papers SET decision = ?3, decision_criteria = ?4, decision_note = ?5, decided_at = ?6
             WHERE collection_id = ?1 AND paper_id = ?2",
            params![collection, paper, s.decision.to_i64(), serde_json::to_string(&s.criteria)?, s.note, decided_at],
        )?;
        if changed == 0 {
            bail!("The paper is no longer in this collection.");
        }
        self.conn.execute("UPDATE papers SET modified_at = ?2 WHERE id = ?1", params![paper, now])?;
        self.touch();
        Ok(())
    }

    /// Sets the decision of several papers at once, keeping their criteria and notes.
    pub fn set_decision(&mut self, collection: i64, papers: &[i64], decision: Decision) -> Result<()> {
        let now = now_string();
        let decided_at = (decision != Decision::Undecided).then_some(now.as_str());
        let tx = self.conn.transaction()?;
        for p in papers {
            tx.execute(
                "UPDATE collection_papers SET decision = ?3, decided_at = ?4 WHERE collection_id = ?1 AND paper_id = ?2",
                params![collection, p, decision.to_i64(), decided_at],
            )?;
        }
        tx.commit()?;
        self.touch();
        Ok(())
    }

    pub fn screening_summary(&self, collection: i64) -> Result<ScreeningSummary> {
        let all = self.screenings(collection)?;
        let criteria = self.criteria(collection)?;
        let mut summary = ScreeningSummary::default();
        for s in all.values() {
            *summary.counts.entry(s.decision).or_insert(0) += 1;
        }
        let excluded: Vec<&Screening> = all.values().filter(|s| s.decision == Decision::Exclude).collect();
        for c in criteria.iter().filter(|c| c.kind == CriterionKind::Exclude) {
            let n = excluded.iter().filter(|s| s.criteria.contains(&c.id)).count();
            summary.excluded_by.push((c.id, n));
        }
        let exclusion_ids: Vec<i64> = criteria.iter().filter(|c| c.kind == CriterionKind::Exclude).map(|c| c.id).collect();
        summary.excluded_without_reason =
            excluded.iter().filter(|s| !s.criteria.iter().any(|c| exclusion_ids.contains(c))).count();
        Ok(summary)
    }
}
