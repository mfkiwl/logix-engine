//! Yosys's JSON netlist (`write_json -compat-int`), the parts the diagram
//! reads.

use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct Netlist {
    #[serde(default)]
    pub modules: IndexMap<String, Module>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Module {
    #[serde(default)]
    pub attributes: IndexMap<String, Value>,
    #[serde(default)]
    pub ports: IndexMap<String, ExtPort>,
    #[serde(default)]
    pub cells: IndexMap<String, Cell>,
    #[serde(default)]
    pub netnames: IndexMap<String, NetName>,
}

/// A module port.
#[derive(Debug, Clone, Deserialize)]
pub struct ExtPort {
    pub direction: String,
    #[serde(default)]
    pub bits: Vec<Bit>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Cell {
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub parameters: IndexMap<String, Value>,
    #[serde(default)]
    pub attributes: IndexMap<String, Value>,
    #[serde(default)]
    pub port_directions: IndexMap<String, String>,
    #[serde(default)]
    pub connections: IndexMap<String, Vec<Bit>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NetName {
    #[serde(default)]
    pub hide_name: Value,
    #[serde(default)]
    pub bits: Vec<Bit>,
}

impl NetName {
    /// A name yosys made up, such as $add$main.v:13$5_Y.
    pub fn hidden(&self) -> bool {
        self.hide_name.as_u64() == Some(1) || self.hide_name.as_str() == Some("1")
    }
}

/// One bit of a signal: a net number, or a fixed value ("0", "1", "x" or
/// "z").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(untagged)]
pub enum Bit {
    Net(u64),
    Const(String),
}

impl Bit {
    pub fn net(&self) -> Option<u64> {
        match self {
            Bit::Net(n) => Some(*n),
            Bit::Const(_) => None,
        }
    }
}

impl Module {
    pub fn is_top(&self) -> bool {
        number(self.attributes.get("top")) == Some(1)
    }
}

/// An attribute or parameter as a number: -compat-int writes small ones as
/// numbers, others as binary text.
pub fn number(v: Option<&Value>) -> Option<u64> {
    match v? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) if !s.is_empty() && s.chars().all(|c| c == '0' || c == '1') => {
            u64::from_str_radix(s.get(s.len().saturating_sub(64)..)?, 2).ok()
        }
        _ => None,
    }
}

/// A text attribute or parameter, such as src or MEMID.
pub fn text(v: Option<&Value>) -> Option<&str> {
    v?.as_str()
}

/// A design name as people write it: yosys's public names have no prefix in
/// JSON, but parameterized modules are named
/// `$paramod$<hash>\<name>` or `$paramod\<name>\<parameters>`.
pub fn plain_name(name: &str) -> String {
    let n = name.trim_start_matches('\\');
    if n.starts_with("$paramod") {
        let mut parts = n.split('\\');
        parts.next();
        if let Some(base) = parts.next() {
            return base.to_string();
        }
    }
    n.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_numbers() {
        assert_eq!(plain_name("main"), "main");
        assert_eq!(plain_name("$paramod$5f3a\\genrom"), "genrom");
        assert_eq!(plain_name("$paramod\\dividerp1\\M=s32'1"), "dividerp1");
        assert_eq!(number(Some(&Value::from(24))), Some(24));
        assert_eq!(
            number(Some(&Value::from("00000000000000000000000000000001"))),
            Some(1)
        );
        assert_eq!(number(Some(&Value::from("main.v:3"))), None);
        let n: Netlist = serde_json::from_str(
            r#"{"modules":{"main":{"attributes":{"top":1},"ports":{"led":{"direction":"output","bits":[2,"0"]}}}}}"#,
        )
        .unwrap();
        assert!(n.modules["main"].is_top());
        assert_eq!(
            n.modules["main"].ports["led"].bits,
            [Bit::Net(2), Bit::Const("0".into())]
        );
    }
}
