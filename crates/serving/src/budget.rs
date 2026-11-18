//! How much text one request may be read into.

use crate::generated::TextLimits;

/// A text budget as a translator reads within it: the most nodes a request's
/// text may decode into, and the deepest it may nest, each positive and
/// within Signal's own bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValidTextBudget {
    pub nodes: usize,
    pub depth: usize,
}

impl ValidTextBudget {
    /// Signal's own bounds, which any text a frame can carry fits.
    pub const MAXIMUM: Self = Self {
        nodes: signal::MAXIMUM_SIGNAL_BYTES,
        depth: signal::MAXIMUM_SIGNAL_DEPTH,
    };
}

impl TryFrom<&TextLimits> for ValidTextBudget {
    type Error = &'static str;

    fn try_from(limits: &TextLimits) -> Result<Self, Self::Error> {
        let nodes = usize::try_from(limits.node_limit.0)
            .ok()
            .filter(|nodes| (1..=Self::MAXIMUM.nodes).contains(nodes))
            .ok_or("text node limit must be positive and within the Signal bound")?;
        let depth = usize::try_from(limits.depth_limit.0)
            .ok()
            .filter(|depth| (1..=Self::MAXIMUM.depth).contains(depth))
            .ok_or("text depth limit must be positive and within the Signal bound")?;
        Ok(Self { nodes, depth })
    }
}

/// A budget one decoding spends.
#[cfg(feature = "text")]
pub trait Budgeting {
    /// A fresh budget for reading one request's text.
    fn budget(&self) -> datom_codec::Budget;
}

#[cfg(feature = "text")]
impl Budgeting for ValidTextBudget {
    fn budget(&self) -> datom_codec::Budget {
        datom_codec::Budget {
            remaining: i64::try_from(self.nodes).expect("a valid node limit fits i64"),
            reader: protos::ReaderBudget::try_from(self.nodes)
                .expect("a valid node limit fits the reader budget"),
            depth: 0,
            maximum_depth: i64::try_from(self.depth).expect("a valid depth limit fits i64"),
        }
    }
}

/// A translator's settings, ready, bound how much ordinary text it reads.
pub trait TextBudgeting {
    fn text_budget(&self) -> ValidTextBudget;
}
