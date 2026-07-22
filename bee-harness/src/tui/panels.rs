//! Panel registry (008-grid-tui, US2): the model-owned named regions rendered beside chat.
//!
//! Insertion-ordered, unique ids, upsert semantics — the same id **replaces** its spec in place
//! (FR-009), a new id **appends** a panel (FR-008). Coalescing (FR-011) is automatic: only the latest
//! spec per id is retained, so a burst of `render_to("m", …)` between two redraws collapses to the
//! last one drawn.
//!
//! **Lifecycle**: panels used to be create-or-replace only, so they accumulated unbounded and each
//! new one shrank the rest. [`PanelOp`] closes that: a script can remove one panel, clear them all,
//! or attach a TTL so a panel expires on its own ([`PanelRegistry::prune`] drops expired entries on
//! each redraw).
//!
//! The data model names this `IndexMap<PanelId, RenderSpec>`; it is realized as an insertion-ordered
//! `Vec` to keep the default build dependency-free (a session holds a handful of panels, so the
//! linear upsert is immaterial and the observable semantics are identical).

use std::time::{Duration, Instant};

use crate::render_spec::{PanelOp, RenderSpec};

/// One live panel: its id, current content, and optional expiry.
#[derive(Debug, Clone)]
pub struct Panel {
    pub id: String,
    pub spec: RenderSpec,
    /// When set, the panel is dropped by [`PanelRegistry::prune`] once this instant passes.
    pub expires_at: Option<Instant>,
}

/// The ordered set of live panels. `Default` is empty (US1 has none).
#[derive(Debug, Default, Clone)]
pub struct PanelRegistry {
    panels: Vec<Panel>,
}

impl PanelRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        PanelRegistry::default()
    }

    /// Apply one lifecycle op (008-grid-tui, US2). `now` anchors any TTL, so callers/tests control
    /// the clock.
    pub fn apply_at(&mut self, op: PanelOp, now: Instant) {
        match op {
            PanelOp::Upsert { id, spec, ttl_ms } => {
                let expires_at = ttl_ms.map(|ms| now + Duration::from_millis(ms));
                self.upsert_with_expiry(id, spec, expires_at);
            }
            PanelOp::Remove { id } => self.remove(&id),
            PanelOp::Clear => self.clear(),
        }
    }

    /// Apply one lifecycle op against the current clock.
    pub fn apply(&mut self, op: PanelOp) {
        self.apply_at(op, Instant::now());
    }

    /// Insert a new panel, or replace an existing one's spec **in place** (FR-008/009). Insertion
    /// order is preserved on replace, so a panel never jumps position when it updates.
    pub fn upsert(&mut self, id: impl Into<String>, spec: RenderSpec) {
        self.upsert_with_expiry(id, spec, None);
    }

    /// [`PanelRegistry::upsert`] with an explicit expiry instant.
    pub fn upsert_with_expiry(
        &mut self,
        id: impl Into<String>,
        spec: RenderSpec,
        expires_at: Option<Instant>,
    ) {
        let id = id.into();
        match self.panels.iter_mut().find(|p| p.id == id) {
            Some(p) => {
                p.spec = spec;
                p.expires_at = expires_at;
            }
            None => self.panels.push(Panel {
                id,
                spec,
                expires_at,
            }),
        }
    }

    /// Remove panel `id`; a no-op when it isn't present.
    pub fn remove(&mut self, id: &str) {
        self.panels.retain(|p| p.id != id);
    }

    /// Remove every panel.
    pub fn clear(&mut self) {
        self.panels.clear();
    }

    /// Drop panels whose TTL has elapsed. Returns how many were dropped (so a caller can redraw).
    pub fn prune(&mut self, now: Instant) -> usize {
        let before = self.panels.len();
        self.panels
            .retain(|p| p.expires_at.is_none_or(|exp| exp > now));
        before - self.panels.len()
    }

    /// Whether any panel carries a TTL — the event loop arms a tick only when this is true.
    pub fn has_expiring(&self) -> bool {
        self.panels.iter().any(|p| p.expires_at.is_some())
    }

    /// The current spec for `id`, if a panel by that name exists.
    pub fn get(&self, id: &str) -> Option<&RenderSpec> {
        self.panels.iter().find(|p| p.id == id).map(|p| &p.spec)
    }

    /// Panels in insertion order as `(id, spec)` — the render order down the column.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &RenderSpec)> {
        self.panels.iter().map(|p| (p.id.as_str(), &p.spec))
    }

    /// Number of live panels.
    pub fn len(&self) -> usize {
        self.panels.len()
    }

    /// Whether there are no panels (US1 state — the panel column is hidden).
    pub fn is_empty(&self) -> bool {
        self.panels.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> RenderSpec {
        RenderSpec::Text {
            content: s.into(),
            style: None,
            bold: false,
            dim: false,
        }
    }

    #[test]
    fn upsert_appends_new_and_replaces_existing_in_place() {
        let mut r = PanelRegistry::new();
        r.upsert("metrics", text("v1"));
        r.upsert("logs", text("l1"));
        r.upsert("metrics", text("v2")); // replace, not duplicate
        assert_eq!(r.len(), 2, "same id replaces — no duplicate panel");
        // Order preserved: metrics stays first even though it was updated last.
        let ids: Vec<&str> = r.iter().map(|(id, _)| id).collect();
        assert_eq!(ids, ["metrics", "logs"]);
        assert_eq!(r.get("metrics"), Some(&text("v2")), "shows the latest spec");
    }

    #[test]
    fn coalescing_keeps_only_the_last_spec_per_id() {
        // A burst of updates to one id between redraws collapses to the last (FR-011).
        let mut r = PanelRegistry::new();
        for i in 0..5 {
            r.upsert("m", text(&format!("v{i}")));
        }
        assert_eq!(r.len(), 1);
        assert_eq!(r.get("m"), Some(&text("v4")));
    }

    #[test]
    fn remove_drops_one_panel_and_is_a_noop_when_absent() {
        let mut r = PanelRegistry::new();
        r.upsert("a", text("1"));
        r.upsert("b", text("2"));
        r.apply(PanelOp::Remove { id: "a".into() });
        assert_eq!(r.len(), 1);
        assert!(r.get("a").is_none());
        r.apply(PanelOp::Remove { id: "nope".into() }); // absent → no panic, no change
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn clear_drops_every_panel() {
        let mut r = PanelRegistry::new();
        for id in ["a", "b", "c"] {
            r.upsert(id, text("x"));
        }
        r.apply(PanelOp::Clear);
        assert!(r.is_empty());
    }

    #[test]
    fn ttl_panels_expire_on_prune_and_others_survive() {
        let now = Instant::now();
        let mut r = PanelRegistry::new();
        r.apply_at(
            PanelOp::Upsert {
                id: "flash".into(),
                spec: text("brief"),
                ttl_ms: Some(100),
            },
            now,
        );
        r.apply_at(
            PanelOp::Upsert {
                id: "keep".into(),
                spec: text("forever"),
                ttl_ms: None,
            },
            now,
        );
        assert!(r.has_expiring());

        // Before the deadline nothing is dropped.
        assert_eq!(r.prune(now + Duration::from_millis(50)), 0);
        assert_eq!(r.len(), 2);

        // After it, only the TTL panel goes.
        assert_eq!(r.prune(now + Duration::from_millis(150)), 1);
        assert_eq!(r.len(), 1);
        assert!(r.get("keep").is_some());
        assert!(!r.has_expiring());
    }

    #[test]
    fn re_upserting_without_a_ttl_clears_a_previous_expiry() {
        let now = Instant::now();
        let mut r = PanelRegistry::new();
        r.apply_at(
            PanelOp::Upsert {
                id: "m".into(),
                spec: text("v1"),
                ttl_ms: Some(10),
            },
            now,
        );
        r.apply_at(
            PanelOp::Upsert {
                id: "m".into(),
                spec: text("v2"),
                ttl_ms: None,
            },
            now,
        );
        assert_eq!(
            r.prune(now + Duration::from_secs(60)),
            0,
            "expiry was lifted"
        );
        assert_eq!(r.get("m"), Some(&text("v2")));
    }
}
