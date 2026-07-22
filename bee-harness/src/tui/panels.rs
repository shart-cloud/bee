//! Panel registry (008-grid-tui, US2 T026): the model-owned named regions rendered beside chat.
//!
//! Insertion-ordered, unique ids, upsert semantics — the same id **replaces** its spec in place
//! (FR-009), a new id **appends** a panel (FR-008). Coalescing (FR-011) is automatic: only the latest
//! spec per id is retained, so a burst of `render_to("m", …)` between two redraws collapses to the
//! last one drawn.
//!
//! The data model names this `IndexMap<PanelId, RenderSpec>`; it is realized as an insertion-ordered
//! `Vec` to keep the default build dependency-free (a session holds a handful of panels, so the
//! linear upsert is immaterial and the observable semantics are identical).

use crate::render_spec::RenderSpec;

/// The ordered set of live panels. `Default` is empty (US1 has none).
#[derive(Debug, Default, Clone)]
pub struct PanelRegistry {
    panels: Vec<(String, RenderSpec)>,
}

impl PanelRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        PanelRegistry::default()
    }

    /// Insert a new panel, or replace an existing one's spec **in place** (FR-008/009). Insertion
    /// order is preserved on replace, so a panel never jumps position when it updates.
    pub fn upsert(&mut self, id: impl Into<String>, spec: RenderSpec) {
        let id = id.into();
        match self.panels.iter_mut().find(|(k, _)| *k == id) {
            Some(slot) => slot.1 = spec,
            None => self.panels.push((id, spec)),
        }
    }

    /// The current spec for `id`, if a panel by that name exists.
    pub fn get(&self, id: &str) -> Option<&RenderSpec> {
        self.panels.iter().find(|(k, _)| k == id).map(|(_, v)| v)
    }

    /// Panels in insertion order as `(id, spec)` — the render order down the column.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &RenderSpec)> {
        self.panels.iter().map(|(k, v)| (k.as_str(), v))
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
    use crate::render_spec::RenderSpec;

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
}
