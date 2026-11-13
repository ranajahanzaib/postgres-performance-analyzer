use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Normalized representation of PostgreSQL's recursive JSON EXPLAIN tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanDocument {
    #[serde(rename = "Plan")]
    pub plan: PlanNode,
    #[serde(rename = "Planning Time", default)]
    pub planning_time_ms: Option<f64>,
    #[serde(rename = "Execution Time", default)]
    pub execution_time_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanNode {
    #[serde(rename = "Node Type")]
    pub node_type: String,
    #[serde(rename = "Relation Name", default)]
    pub relation: Option<String>,
    #[serde(rename = "Schema", default)]
    pub schema: Option<String>,
    #[serde(rename = "Alias", default)]
    pub alias: Option<String>,
    #[serde(rename = "Index Name", default)]
    pub index_name: Option<String>,
    #[serde(rename = "Startup Cost", default)]
    pub startup_cost: Option<f64>,
    #[serde(rename = "Total Cost", default)]
    pub total_cost: Option<f64>,
    #[serde(rename = "Plan Rows", default)]
    pub plan_rows: Option<f64>,
    #[serde(rename = "Plan Width", default)]
    pub plan_width: Option<f64>,
    #[serde(rename = "Actual Startup Time", default)]
    pub actual_startup_time_ms: Option<f64>,
    #[serde(rename = "Actual Total Time", default)]
    pub actual_total_time_ms: Option<f64>,
    #[serde(rename = "Actual Rows", default)]
    pub actual_rows_per_loop: Option<f64>,
    #[serde(rename = "Actual Loops", default)]
    pub actual_loops: Option<f64>,
    #[serde(rename = "Rows Removed by Filter", default)]
    pub rows_removed_by_filter_per_loop: Option<f64>,
    #[serde(rename = "Filter", default)]
    pub filter: Option<String>,
    #[serde(rename = "Index Cond", default)]
    pub index_condition: Option<String>,
    #[serde(rename = "Sort Method", default)]
    pub sort_method: Option<String>,
    #[serde(rename = "Sort Space Used", default)]
    pub sort_space_used_kb: Option<f64>,
    #[serde(rename = "Sort Space Type", default)]
    pub sort_space_type: Option<String>,
    #[serde(rename = "Hash Batches", default)]
    pub hash_batches: Option<f64>,
    #[serde(rename = "Shared Hit Blocks", default)]
    pub shared_hit_blocks: Option<f64>,
    #[serde(rename = "Shared Read Blocks", default)]
    pub shared_read_blocks: Option<f64>,
    #[serde(rename = "Shared Dirtied Blocks", default)]
    pub shared_dirtied_blocks: Option<f64>,
    #[serde(rename = "Shared Written Blocks", default)]
    pub shared_written_blocks: Option<f64>,
    #[serde(rename = "Temp Read Blocks", default)]
    pub temp_read_blocks: Option<f64>,
    #[serde(rename = "Temp Written Blocks", default)]
    pub temp_written_blocks: Option<f64>,
    #[serde(rename = "Plans", default)]
    pub children: Vec<PlanNode>,
    /// Preserve fields from newer PostgreSQL versions and node types.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanFinding {
    pub rule_id: &'static str,
    pub severity: &'static str,
    pub confidence: &'static str,
    pub node_type: String,
    pub relation: Option<String>,
    pub evidence: String,
    pub next_step: &'static str,
}

impl PlanDocument {
    pub fn parse(value: Value, keep_raw: bool) -> Result<Self> {
        let raw = keep_raw.then(|| value.clone());
        let root = value
            .as_array()
            .and_then(|items| items.first())
            .context("EXPLAIN JSON must be a non-empty array")?;
        let mut doc: Self = serde_json::from_value(root.clone())
            .context("invalid PostgreSQL JSON EXPLAIN document")?;
        doc.raw = raw;
        Ok(doc)
    }

    pub fn findings(&self) -> Vec<PlanFinding> {
        let mut out = Vec::new();
        self.plan.visit(&mut |n| {
            let relation = n.relation.clone();
            if n.node_type.contains("Seq Scan")
                && let (Some(estimated), Some(actual), Some(removed)) =
                    (n.plan_rows, n.actual_rows_per_loop, n.rows_removed_by_filter_per_loop)
            {
                    let observed = actual + removed;
                    if observed >= 10_000.0 && actual / observed.max(1.0) < 0.1 && n.filter.is_some() {
                        out.push(PlanFinding {
                            rule_id: "SEQ_SCAN_HIGH_FILTER_COST",
                            severity: "medium",
                            confidence: "medium",
                            node_type: n.node_type.clone(),
                            relation: relation.clone(),
                            evidence: format!("{} candidate rows scanned across loops; {:.1}% returned after filtering (planner estimate: {:.0} rows)", observed, actual / observed.max(1.0) * 100.0, estimated),
                            next_step: "Inspect predicate selectivity and existing indexes; test a candidate index under the same workload. A sequential scan can still be optimal for broad scans.",
                        });
                    }
            }
            if let (Some(est), Some(actual_total)) = (n.plan_rows, n.actual_rows_per_loop) {
                let loops = n.actual_loops.unwrap_or(1.0);
                if (est >= 10.0 && actual_total > est * 10.0)
                    || (actual_total >= 10.0 && est > actual_total * 10.0)
                {
                    out.push(PlanFinding {
                        rule_id: "ROW_ESTIMATE_MISMATCH",
                        severity: "medium",
                        confidence: "high",
                        node_type: n.node_type.clone(),
                        relation: relation.clone(),
                        evidence: format!("estimated {est:.0} rows per loop; observed about {actual_total:.0} rows per loop across {loops:.0} loop(s)"),
                        next_step: "Check ANALYZE freshness, skew, correlated predicates, and parameter sensitivity; this evidence does not identify the cause.",
                    });
                }
            }
            if n.node_type.contains("Sort") && (n.sort_space_type.as_deref() == Some("Disk") || n.temp_written_blocks.unwrap_or(0.0) > 0.0) {
                out.push(PlanFinding {
                    rule_id: "SORT_SPILL_TO_DISK",
                    severity: "medium",
                    confidence: "high",
                    node_type: n.node_type.clone(),
                    relation: relation.clone(),
                    evidence: format!("sort method {:?}, space {:?} {:?}, temporary blocks written {:?}", n.sort_method, n.sort_space_used_kb, n.sort_space_type, n.temp_written_blocks),
                    next_step: "Check whether filtering or index ordering can reduce sort input. Consider work_mem only with concurrent-session memory use in mind.",
                });
            }
            if n.node_type.contains("Hash") && n.hash_batches.is_some_and(|b| b > 1.0) {
                out.push(PlanFinding {
                    rule_id: "HASH_SPILL_MULTIPLE_BATCHES",
                    severity: "medium",
                    confidence: "medium",
                    node_type: n.node_type.clone(),
                    relation: relation.clone(),
                    evidence: format!("hash operation used {} batches", n.hash_batches.unwrap_or_default()),
                    next_step: "Check input cardinality estimates and memory pressure; a multi-batch hash is evidence of spill behavior, not a universal tuning prescription.",
                });
            }
            if n.node_type == "Nested Loop" && n.children.iter().any(|c| c.actual_loops.unwrap_or(0.0) > 1000.0) {
                let loops = n.children.iter().filter_map(|c| c.actual_loops).fold(0.0, f64::max);
                out.push(PlanFinding {
                    rule_id: "REPEATED_NESTED_LOOP_CHILD",
                    severity: "medium",
                    confidence: "medium",
                    node_type: n.node_type.clone(),
                    relation: relation.clone(),
                    evidence: format!("a child node executed up to {loops:.0} times"),
                    next_step: "Inspect join cardinality and child access cost; repeated loops can be appropriate for small inputs.",
                });
            }
        });
        out
    }
}

impl PlanNode {
    pub fn visit(&self, f: &mut impl FnMut(&PlanNode)) {
        f(self);
        for child in &self.children {
            child.visit(f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_recursive_unknown_nodes_and_loop_metrics() {
        let raw = serde_json::json!([{"Plan":{"Node Type":"Custom Scan","Plan Rows":2,"Actual Rows":3,"Actual Loops":4,"Plans":[{"Node Type":"Index Scan","Index Name":"idx_x","Actual Rows":1,"Actual Loops":5,"Custom Field":"kept"}]},"Planning Time":1.2,"Execution Time":5.5}]);
        let p = PlanDocument::parse(raw, true).unwrap();
        assert_eq!(
            p.plan.children[0].actual_rows_per_loop.unwrap()
                * p.plan.children[0].actual_loops.unwrap(),
            5.0
        );
        assert_eq!(
            p.plan.children[0].extra.get("Custom Field").unwrap(),
            "kept"
        );
        assert!(p.raw.is_some());
    }
    #[test]
    fn recognizes_filter_heavy_scan_and_disk_sort() {
        let raw = serde_json::json!([{"Plan":{"Node Type":"Sort","Sort Method":"external merge","Sort Space Type":"Disk","Plans":[{"Node Type":"Seq Scan","Relation Name":"orders","Plan Rows":10,"Actual Rows":20,"Actual Loops":1,"Rows Removed by Filter":20000,"Filter":"tenant_id = 1"}]}}]);
        let p = PlanDocument::parse(raw, false).unwrap();
        let ids: Vec<_> = p.findings().into_iter().map(|f| f.rule_id).collect();
        assert!(ids.contains(&"SEQ_SCAN_HIGH_FILTER_COST"));
        assert!(ids.contains(&"SORT_SPILL_TO_DISK"));
    }
}
