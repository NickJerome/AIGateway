//! Committed telemetry read model; public envelopes are typed, plugin data remains extensible.
use super::{Kind, ListCursor, Telemetry};
use anyhow::{Result, anyhow, bail};
use rusqlite::params;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
type UsageGroup = (Vec<Value>, u64, BTreeMap<String, f64>);

#[derive(Debug, Serialize)]
pub struct Record {
    pub id: i64,
    pub plugin_id: String,
    pub time: i64,
    pub request_id: String,
    pub data: Value,
}
#[derive(Debug, Serialize)]
pub struct Page {
    pub rows: Vec<Record>,
    pub has_more: bool,
}
impl Telemetry {
    pub async fn query(&self, plugin: String, since: i64, group: Vec<String>) -> Result<Value> {
        if group.len() > 4 {
            bail!("groupBy accepts at most four fields");
        }
        self.0
            .db
            .run(move |c| {
                let mut statement =
                    c.prepare("SELECT value FROM usage WHERE plugin=?1 AND time>=?2 ORDER BY id")?;
                let mut groups: BTreeMap<String, UsageGroup> = BTreeMap::new();
                let mut found = std::collections::HashSet::new();
                let mut truncated = false;
                for row in statement.query_map(params![plugin, since], |r| r.get::<_, String>(0))? {
                    let record: Value = serde_json::from_str(&row?)?;
                    let object = record
                        .as_object()
                        .ok_or_else(|| anyhow!("invalid stored usage"))?;
                    found.extend(object.keys().cloned());
                    let values = group.iter().map(|k| record[k].clone()).collect::<Vec<_>>();
                    if values.iter().any(|v| v.is_number()) {
                        bail!("numeric usage fields cannot be grouped");
                    }
                    let key = serde_json::to_string(&values)?;
                    if !groups.contains_key(&key) && groups.len() >= 1000 {
                        truncated = true;
                        continue;
                    }
                    let entry = groups.entry(key).or_insert((values, 0, BTreeMap::new()));
                    entry.1 += 1;
                    for (k, v) in object {
                        if !group.contains(k)
                            && let Some(n) = v.as_f64()
                        {
                            let sum = entry.2.entry(k.clone()).or_default();
                            *sum += n;
                            if !sum.is_finite() {
                                bail!("usage aggregate exceeds finite number range");
                            }
                        }
                    }
                }
                if !groups.is_empty() && group.iter().any(|k| !found.contains(k)) {
                    bail!("unknown usage grouping field");
                }
                let rows = groups
                    .into_values()
                    .map(|(values, count, sums)| {
                        let mut row = serde_json::Map::new();
                        for (k, v) in group.iter().zip(values) {
                            row.insert(k.clone(), v);
                        }
                        json!({"group": row, "count": count, "sums": sums})
                    })
                    .collect::<Vec<_>>();
                Ok(json!({"rows":rows,"truncated":truncated}))
            })
            .await
    }
    /// Forward cursors preserve the existing API; backward cursors start at newest.
    pub async fn list(
        &self,
        plugin: Option<String>,
        kind: Kind,
        since: i64,
        cursor: ListCursor,
        limit: usize,
    ) -> Result<Page> {
        if !(1..=1000).contains(&limit) {
            bail!("limit must be 1–1000");
        }
        let (table, id) = match kind {
            Kind::Logs => ("logs", "id"),
            Kind::Usage => ("usage", "id"),
            Kind::Traces => ("traces", "rowid"),
        };
        let (comparison, order, boundary) = match cursor {
            ListCursor::After(id) => (">", "ASC", id),
            ListCursor::Before(id) => ("<", "DESC", id),
        };
        self.0
            .db
            .run(move |c| {
                // Table, column, comparison and order come only from the matches above.
                let sql = format!(
                    "SELECT {id}, time, request, value, plugin FROM {table}
                 WHERE (?1 IS NULL OR plugin=?1) AND time>=?2 AND {id}{comparison}?3
                 ORDER BY {id} {order} LIMIT ?4"
                );
                let mut statement = c.prepare(&sql)?;
                let mut query =
                    statement.query(params![plugin, since, boundary, (limit + 1) as i64])?;
                let mut rows = Vec::new();
                while let Some(row) = query.next()? {
                    rows.push(Record {
                        id: row.get(0)?,
                        plugin_id: row.get(4)?,
                        time: row.get(1)?,
                        request_id: row.get(2)?,
                        data: serde_json::from_str(&row.get::<_, String>(3)?)?,
                    });
                }
                let has_more = rows.len() > limit;
                rows.truncate(limit);
                Ok(Page { rows, has_more })
            })
            .await
    }
    pub async fn detail(&self, plugin: String, request: String) -> Result<Value> {
        self.0
            .db
            .run(move |c| {
                let text: String = c.query_row(
                    "SELECT value FROM traces WHERE plugin=?1 AND request=?2",
                    params![plugin, request],
                    |r| r.get(0),
                )?;
                Ok(serde_json::from_str(&text)?)
            })
            .await
    }
    pub async fn chunk(
        &self,
        plugin: String,
        request: String,
        side: String,
        seq: u64,
    ) -> Result<Option<Vec<u8>>> {
        use rusqlite::OptionalExtension;
        self.0.db.run(move|c|Ok(c.query_row("SELECT body FROM trace_chunks WHERE plugin=?1 AND request=?2 AND side=?3 AND seq=?4",params![plugin,request,side,seq as i64],|r|r.get(0)).optional()?)).await
    }
}
