use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Durability {
    #[default]
    Durable,
    Balanced,
    Performance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KvEntry {
    pub namespace: String,
    #[serde(with = "serde_bytes")]
    pub key: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub value: Vec<u8>,
    #[serde(with = "crate::version")]
    pub version: u64,
    pub expires_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonEntry {
    pub space: String,
    pub id: String,
    pub value: Value,
    #[serde(with = "crate::version")]
    pub version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "model", rename_all = "snake_case")]
pub enum BatchOperation {
    KvSet {
        namespace: String,
        #[serde(with = "serde_bytes")]
        key: Vec<u8>,
        #[serde(with = "serde_bytes")]
        value: Vec<u8>,
        #[serde(default, with = "crate::version::optional")]
        if_version: Option<u64>,
        expires_at_ms: Option<i64>,
    },
    KvDelete {
        namespace: String,
        #[serde(with = "serde_bytes")]
        key: Vec<u8>,
        #[serde(default, with = "crate::version::optional")]
        if_version: Option<u64>,
    },
    JsonPut {
        space: String,
        id: String,
        value: Value,
        #[serde(default, with = "crate::version::optional")]
        if_version: Option<u64>,
    },
    JsonDelete {
        space: String,
        id: String,
        #[serde(default, with = "crate::version::optional")]
        if_version: Option<u64>,
    },
    GraphSetNode {
        graph: String,
        id: String,
        kind: String,
        metadata: Value,
    },
    GraphDeleteNode {
        graph: String,
        id: String,
    },
    GraphAddEdge {
        graph: String,
        source: String,
        target: String,
        kind: String,
    },
    GraphRemoveEdge {
        graph: String,
        source: String,
        target: String,
        kind: String,
    },
    FtsIndex {
        collection: String,
        doc_id: String,
        content: String,
    },
    FtsDelete {
        collection: String,
        doc_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationResult {
    #[serde(default, with = "crate::version::optional")]
    pub version: Option<u64>,
    pub deleted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Outgoing,
    Incoming,
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub graph: String,
    pub id: String,
    pub kind: String,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub graph: String,
    pub source: String,
    pub target: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FtsHit {
    pub collection: String,
    pub doc_id: String,
    pub snippet: String,
    pub rank: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DatabaseEvent {
    KvMutated {
        namespace: String,
        #[serde(with = "serde_bytes")]
        key: Vec<u8>,
        version: Option<u64>,
        deleted: bool,
    },
    JsonMutated {
        space: String,
        id: String,
        version: Option<u64>,
        deleted: bool,
    },
    GraphNodeMutated {
        graph: String,
        id: String,
        deleted: bool,
    },
    GraphEdgeMutated {
        graph: String,
        source: String,
        target: String,
        kind: String,
        deleted: bool,
    },
    FtsMutated {
        collection: String,
        doc_id: String,
        deleted: bool,
    },
}

impl DatabaseEvent {
    #[must_use]
    pub fn matches_filter(&self, filter: Option<&str>) -> bool {
        let Some(filter) = filter else { return true };
        if filter.is_empty() || filter == "*" {
            return true;
        }
        match self {
            Self::KvMutated { namespace, .. } => namespace == filter,
            Self::JsonMutated { space, .. } => space == filter,
            Self::GraphNodeMutated { graph, .. } | Self::GraphEdgeMutated { graph, .. } => {
                graph == filter
            }
            Self::FtsMutated { collection, .. } => collection == filter,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SqlQueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub rows_affected: u64,
}
