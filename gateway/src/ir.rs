//! Offset-indexed JSON. Unchanged ranges remain slices of the incoming body.
use anyhow::{Result, anyhow, bail};
use bytes::Bytes;
use serde_json::{Value, json};
#[derive(Clone)]
enum Kind {
    Scalar,
    Object(Vec<(String, Node)>),
    Array(Vec<Node>),
}
#[derive(Clone)]
struct Node {
    start: usize,
    end: usize,
    kind: Kind,
    replacement: Option<Value>,
    reshaped: bool,
}
impl Node {
    fn new(value: Value) -> Self {
        Self {
            start: 0,
            end: 0,
            kind: Kind::Scalar,
            replacement: Some(value),
            reshaped: false,
        }
    }
    fn child(&self, key: &str) -> Option<&Self> {
        match &self.kind {
            Kind::Object(items) => items.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v),
            Kind::Array(items) => key.parse::<usize>().ok().and_then(|i| items.get(i)),
            _ => None,
        }
    }
    fn child_mut(&mut self, key: &str) -> Option<&mut Self> {
        match &mut self.kind {
            Kind::Object(items) => items
                .iter_mut()
                .rev()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v),
            Kind::Array(items) => key.parse::<usize>().ok().and_then(|i| items.get_mut(i)),
            _ => None,
        }
    }
    fn value(&self, raw: &Bytes) -> Result<Value> {
        let mut pieces = Vec::new();
        self.emit(raw, &mut pieces)?;
        let all = pieces.concat();
        Ok(serde_json::from_slice(&all)?)
    }
    fn emit(&self, raw: &Bytes, out: &mut Vec<Bytes>) -> Result<()> {
        if let Some(v) = &self.replacement {
            out.push(Bytes::from(serde_json::to_vec(v)?));
            return Ok(());
        }
        if self.reshaped {
            match &self.kind {
                Kind::Object(items) => {
                    out.push(Bytes::from_static(b"{"));
                    for (i, (key, value)) in items.iter().enumerate() {
                        if i > 0 {
                            out.push(Bytes::from_static(b","));
                        }
                        out.push(Bytes::from(serde_json::to_vec(key)?));
                        out.push(Bytes::from_static(b":"));
                        value.emit(raw, out)?;
                    }
                    out.push(Bytes::from_static(b"}"));
                }
                Kind::Array(items) => {
                    out.push(Bytes::from_static(b"["));
                    for (i, value) in items.iter().enumerate() {
                        if i > 0 {
                            out.push(Bytes::from_static(b","));
                        }
                        value.emit(raw, out)?;
                    }
                    out.push(Bytes::from_static(b"]"));
                }
                Kind::Scalar => unreachable!(),
            }
            return Ok(());
        }
        let children: Vec<&Node> = match &self.kind {
            Kind::Object(items) => items.iter().map(|(_, v)| v).collect(),
            Kind::Array(items) => items.iter().collect(),
            Kind::Scalar => vec![],
        };
        let mut at = self.start;
        for child in children {
            if at < child.start {
                out.push(raw.slice(at..child.start));
            }
            child.emit(raw, out)?;
            at = child.end;
        }
        if at < self.end {
            out.push(raw.slice(at..self.end));
        }
        Ok(())
    }
    fn read(&self, raw: &Bytes, path: &[String]) -> Result<Value> {
        if let Some(v) = &self.replacement {
            if path.last().map(String::as_str) == Some("length")
                && let Some(Value::Array(a)) = json_path(v, &path[..path.len() - 1])
            {
                return Ok(json!({"kind":"scalar","value":a.len()}));
            }
            return describe_json(json_path(v, path));
        }
        if let Some((key, tail)) = path.split_first() {
            if let Kind::Array(items) = &self.kind
                && key == "length"
                && tail.is_empty()
            {
                return Ok(json!({"kind":"scalar","value":items.len()}));
            }
            return self
                .child(key)
                .map_or(Ok(json!({"kind":"missing"})), |v| v.read(raw, tail));
        }
        Ok(match &self.kind {
            Kind::Scalar => {
                json!({"kind":"scalar","value":serde_json::from_slice::<Value>(&raw[self.start..self.end])?})
            }
            Kind::Array(items) => {
                json!({"kind":"array","length":items.len(),"keys":(0..items.len()).map(|n|n.to_string()).collect::<Vec<_>>()})
            }
            Kind::Object(items) => {
                let mut keys = Vec::new();
                for (k, _) in items {
                    if !keys.contains(k) {
                        keys.push(k.clone());
                    }
                }
                json!({"kind":"object","keys":keys})
            }
        })
    }
    fn edit(&mut self, path: &[String], value: Option<Value>) -> Result<()> {
        if path.is_empty() {
            self.replacement = Some(value.unwrap_or(Value::Null));
            return Ok(());
        }
        if let Some(replaced) = &mut self.replacement {
            return edit_json(replaced, path, value);
        }
        let (key, tail) = path.split_first().unwrap();
        if !tail.is_empty() {
            return self
                .child_mut(key)
                .ok_or_else(|| anyhow!("IR parent does not exist"))?
                .edit(tail, value);
        }
        match &mut self.kind {
            Kind::Object(items) => {
                if let Some(value) = value {
                    if let Some((_, v)) = items.iter_mut().rev().find(|(k, _)| k == key) {
                        v.replacement = Some(value);
                    } else {
                        items.push((key.clone(), Node::new(value)));
                        self.reshaped = true;
                    }
                } else {
                    items.retain(|(k, _)| k != key);
                    self.reshaped = true;
                }
            }
            Kind::Array(items) => {
                if key == "length" {
                    let len = value
                        .as_ref()
                        .and_then(Value::as_u64)
                        .ok_or_else(|| anyhow!("invalid array length"))?
                        as usize;
                    if len > 1_000_000 {
                        bail!("IR array length exceeds limit");
                    }
                    items.resize_with(len, || Node::new(Value::Null));
                    self.reshaped = true;
                } else {
                    let i = key
                        .parse::<usize>()
                        .map_err(|_| anyhow!("invalid array index"))?;
                    if i > 1_000_000 {
                        bail!("IR array index exceeds limit");
                    }
                    if i >= items.len() {
                        items.resize_with(i + 1, || Node::new(Value::Null));
                        self.reshaped = true;
                    }
                    items[i].replacement = Some(value.unwrap_or(Value::Null));
                }
            }
            Kind::Scalar => bail!("cannot edit a scalar child"),
        }
        Ok(())
    }
}
fn json_path<'a>(mut v: &'a Value, path: &[String]) -> Option<&'a Value> {
    for key in path {
        v = if let Value::Array(a) = v {
            a.get(key.parse::<usize>().ok()?)?
        } else {
            v.get(key)?
        };
    }
    Some(v)
}
fn describe_json(v: Option<&Value>) -> Result<Value> {
    Ok(match v {
        None => json!({"kind":"missing"}),
        Some(Value::Array(a)) => {
            json!({"kind":"array","length":a.len(),"keys":(0..a.len()).map(|n|n.to_string()).collect::<Vec<_>>()})
        }
        Some(Value::Object(o)) => json!({"kind":"object","keys":o.keys().collect::<Vec<_>>()}),
        Some(v) => json!({"kind":"scalar","value":v}),
    })
}
fn edit_json(v: &mut Value, path: &[String], value: Option<Value>) -> Result<()> {
    if path.is_empty() {
        *v = value.unwrap_or(Value::Null);
        return Ok(());
    }
    let (key, rest) = path.split_first().unwrap();
    if rest.is_empty() {
        match v {
            Value::Object(o) => {
                if let Some(value) = value {
                    o.insert(key.clone(), value);
                } else {
                    o.shift_remove(key);
                }
            }
            Value::Array(a) => {
                if key == "length" {
                    let n = value
                        .and_then(|v| v.as_u64())
                        .ok_or_else(|| anyhow!("invalid length"))?
                        as usize;
                    if n > 1_000_000 {
                        bail!("array too large");
                    }
                    a.resize(n, Value::Null);
                } else {
                    let i = key.parse::<usize>()?;
                    if i > 1_000_000 {
                        bail!("array too large");
                    }
                    if i >= a.len() {
                        a.resize(i + 1, Value::Null);
                    }
                    a[i] = value.unwrap_or(Value::Null);
                }
            }
            _ => bail!("not a container"),
        };
        return Ok(());
    }
    let child = match v {
        Value::Array(a) => a.get_mut(key.parse::<usize>()?),
        Value::Object(o) => o.get_mut(key),
        _ => None,
    }
    .ok_or_else(|| anyhow!("missing parent"))?;
    edit_json(child, rest, value)
}
struct Parser<'a> {
    charge: Option<crate::network::Reservation>,
    raw: &'a [u8],
    pos: usize,
}
impl Parser<'_> {
    fn ws(&mut self) {
        while self
            .raw
            .get(self.pos)
            .is_some_and(|c| matches!(c, b' ' | b'\t' | b'\r' | b'\n'))
        {
            self.pos += 1;
        }
    }
    fn string(&mut self) -> Result<String> {
        let start = self.pos;
        self.pos += 1;
        loop {
            match self.raw.get(self.pos) {
                Some(b'"') => {
                    self.pos += 1;
                    break;
                }
                Some(b'\\') => self.pos += 2,
                Some(_) => self.pos += 1,
                None => bail!("unterminated JSON string"),
            }
        }
        Ok(serde_json::from_slice(&self.raw[start..self.pos])?)
    }
    fn node(&mut self, depth: usize) -> Result<Node> {
        if let Some(charge) = &mut self.charge {
            charge.grow(2 * std::mem::size_of::<Node>())?;
        }
        if depth > 128 {
            bail!("JSON depth exceeds 128");
        }
        self.ws();
        let start = self.pos;
        let kind = match self.raw.get(self.pos) {
            Some(b'{') => {
                self.pos += 1;
                self.ws();
                let mut entries = vec![];
                if self.raw.get(self.pos) != Some(&b'}') {
                    loop {
                        self.ws();
                        if self.raw.get(self.pos) != Some(&b'"') {
                            bail!("expected JSON key");
                        }
                        let key = self.string()?;
                        if let Some(charge) = &mut self.charge {
                            charge.grow(key.capacity())?;
                        }
                        self.ws();
                        if self.raw.get(self.pos) != Some(&b':') {
                            bail!("expected colon");
                        }
                        self.pos += 1;
                        entries.push((key, self.node(depth + 1)?));
                        self.ws();
                        if self.raw.get(self.pos) != Some(&b',') {
                            break;
                        }
                        self.pos += 1;
                    }
                }
                if self.raw.get(self.pos) != Some(&b'}') {
                    bail!("expected object end");
                }
                self.pos += 1;
                Kind::Object(entries)
            }
            Some(b'[') => {
                self.pos += 1;
                self.ws();
                let mut entries = vec![];
                if self.raw.get(self.pos) != Some(&b']') {
                    loop {
                        entries.push(self.node(depth + 1)?);
                        self.ws();
                        if self.raw.get(self.pos) != Some(&b',') {
                            break;
                        }
                        self.pos += 1;
                    }
                }
                if self.raw.get(self.pos) != Some(&b']') {
                    bail!("expected array end");
                }
                self.pos += 1;
                Kind::Array(entries)
            }
            Some(b'"') => {
                self.string()?;
                Kind::Scalar
            }
            Some(_) => {
                while self.raw.get(self.pos).is_some_and(|c| {
                    !matches!(c, b' ' | b'\t' | b'\r' | b'\n') && !b",]}".contains(c)
                }) {
                    self.pos += 1;
                }
                if self.pos == start {
                    bail!("invalid JSON value");
                }
                let _: Value = serde_json::from_slice(&self.raw[start..self.pos])?;
                Kind::Scalar
            }
            None => bail!("missing JSON value"),
        };
        Ok(Node {
            start,
            end: self.pos,
            kind,
            replacement: None,
            reshaped: false,
        })
    }
}
pub struct Document {
    charge: Option<crate::network::Reservation>,
    raw: Bytes,
    root: Node,
}
impl Document {
    pub fn parse(raw: Bytes) -> Result<Self> {
        Self::parse_budget(raw, None)
    }
    pub fn parse_budget(raw: Bytes, budget: Option<&crate::network::Budget>) -> Result<Self> {
        let mut p = Parser {
            raw: &raw,
            pos: 0,
            charge: budget.map(|b| b.reserve(0)).transpose()?,
        };
        let root = p.node(0)?;
        p.ws();
        if p.pos != raw.len() {
            bail!("trailing JSON input");
        }
        if !matches!(root.kind, Kind::Object(_)) {
            bail!("IR requires an object");
        }
        let charge = p.charge;
        Ok(Self { raw, root, charge })
    }
    pub fn read(&self, path: &[String]) -> Result<Value> {
        self.root.read(&self.raw, path)
    }
    pub fn edit(&mut self, path: &[String], value: Option<Value>) -> Result<()> {
        let mut allocation = value
            .as_ref()
            .map(|v| serde_json::to_vec(v).map(|b| b.len().saturating_mul(4) + 128))
            .transpose()?
            .unwrap_or(0);
        if let Some((key, parent)) = path.split_last() {
            let description = self.root.read(&self.raw, parent)?;
            if description["kind"] == "array" {
                let previous = description["length"].as_u64().unwrap_or(0) as usize;
                let next = if key == "length" {
                    value.as_ref().and_then(Value::as_u64).unwrap_or(0) as usize
                } else {
                    key.parse::<usize>().unwrap_or(0).saturating_add(1)
                };
                allocation = allocation.saturating_add(
                    next.saturating_sub(previous)
                        .saturating_mul(std::mem::size_of::<Node>() * 2),
                );
            }
        }
        if let Some(charge) = &mut self.charge {
            charge.grow(allocation)?;
        }
        self.root.edit(path, value)
    }
    pub fn snapshot(&self) -> Result<Vec<Bytes>> {
        let mut pieces = vec![];
        if self.root.start > 0 {
            pieces.push(self.raw.slice(..self.root.start));
        }
        self.root.emit(&self.raw, &mut pieces)?;
        if self.root.end < self.raw.len() {
            pieces.push(self.raw.slice(self.root.end..));
        }
        Ok(pieces)
    }
    pub fn snapshot_charged(&self, budget: &crate::network::Budget) -> Result<Vec<Bytes>> {
        let start = self.raw.as_ptr() as usize;
        let end = start + self.raw.len();
        self.snapshot()?
            .into_iter()
            .map(|part| {
                let ptr = part.as_ptr() as usize;
                if ptr >= start && ptr + part.len() <= end {
                    Ok(part)
                } else {
                    Ok(crate::network::Chunk::new(part, budget)?.bytes)
                }
            })
            .collect()
    }
    pub fn materialize(&self) -> Result<Value> {
        self.root.value(&self.raw)
    }
}
