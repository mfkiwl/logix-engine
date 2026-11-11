//! The diagram's symbols: each block's shape, size and port positions.
//! Based on netlistsvg's default skin (MIT), changed as the prototype
//! decided: pins point right, the way the signal goes; adders, subtractors
//! and comparators are rounded squares with their ports on the edge; a
//! split's input and a join's output sit near the top of the bar; and
//! labels, for signals drawn without wires.

/// A symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Mux,
    And,
    Nand,
    Or,
    Nor,
    Xor,
    Xnor,
    Not,
    Add,
    Sub,
    Eq,
    Lt,
    Le,
    Ge,
    Gt,
    Dff,
    Dffn,
    /// A box with its inputs on the left and outputs on the right, as tall
    /// as its ports need: modules, FPGA blocks and every other cell.
    Generic,
    /// A module input: one of the FPGA's pins in the top module.
    PinIn,
    /// A module output.
    PinOut,
    /// A fixed value too wide for a label.
    Constant,
    /// A bus split: one bus in, parts of it out.
    Split,
    /// A bus join: parts in, one bus out.
    Join,
    /// A label a signal comes from (its text ends at the port), by size.
    LabelIn(u8),
    /// A label a signal goes to, by size.
    LabelOut(u8),
}

/// Label widths, by size: the longest text each fits at 6 units per
/// character (`label_size`).
pub const LABEL_WIDTHS: [(u8, f64); 4] = [(1, 18.0), (2, 36.0), (3, 60.0), (4, 96.0)];

/// The smallest label that fits `text`.
pub fn label_size(text: &str) -> u8 {
    let need = text.chars().count() as f64 * 6.0 + 6.0;
    LABEL_WIDTHS
        .iter()
        .find(|(_, w)| need <= *w)
        .map_or(4, |(n, _)| *n)
}

fn label_width(n: u8) -> f64 {
    LABEL_WIDTHS
        .iter()
        .find(|(k, _)| *k == n)
        .map_or(96.0, |(_, w)| *w)
}

/// The space between a generic block's ports, and a split's or join's.
pub const PORT_GAP: f64 = 20.0;
/// The first port's height on a generic block, split or join.
pub const FIRST_PORT_Y: f64 = 10.0;

impl Shape {
    /// The symbol for a yosys cell type, given the cell's ports: a fixed
    /// symbol only when it has a place for every port.
    pub fn for_cell<'a>(ty: &str, mut ports: impl Iterator<Item = &'a str>) -> Shape {
        let shape = match ty {
            "$mux" | "$pmux" | "$_MUX_" => Shape::Mux,
            "$and" | "$logic_and" | "$_AND_" => Shape::And,
            "$nand" | "$_NAND_" | "$_ANDNOT_" => Shape::Nand,
            "$or" | "$logic_or" | "$_OR_" => Shape::Or,
            "$nor" | "$reduce_nor" | "$_NOR_" | "$_ORNOT_" => Shape::Nor,
            "$xor" | "$reduce_xor" | "$_XOR_" => Shape::Xor,
            "$xnor" | "$reduce_xnor" | "$_XNOR_" => Shape::Xnor,
            "$not" | "$logic_not" | "$_NOT_" => Shape::Not,
            "$add" => Shape::Add,
            "$sub" => Shape::Sub,
            "$eq" => Shape::Eq,
            "$lt" => Shape::Lt,
            "$le" => Shape::Le,
            "$ge" => Shape::Ge,
            "$gt" => Shape::Gt,
            "$dff" | "$_DFF_" | "$_DFF_P_" => Shape::Dff,
            "$_DFF_N_" => Shape::Dffn,
            _ => Shape::Generic,
        };
        if shape != Shape::Generic && ports.all(|p| shape.port(p).is_some()) {
            shape
        } else {
            Shape::Generic
        }
    }

    /// A fixed symbol's width and height. Generic blocks, splits and joins
    /// grow with their ports (`Node::size`).
    pub fn size(self) -> (f64, f64) {
        match self {
            Shape::Mux => (20.0, 40.0),
            Shape::And | Shape::Nand | Shape::Or => (30.0, 25.0),
            Shape::Nor | Shape::Xor | Shape::Xnor => (33.0, 25.0),
            Shape::Not => (30.0, 20.0),
            Shape::Add | Shape::Sub | Shape::Eq | Shape::Lt | Shape::Le | Shape::Ge | Shape::Gt => {
                (25.0, 25.0)
            }
            Shape::Dff | Shape::Dffn | Shape::Generic => (30.0, 40.0),
            Shape::PinIn | Shape::PinOut | Shape::Constant => (30.0, 20.0),
            Shape::Split => (5.0, 40.0),
            Shape::Join => (4.0, 40.0),
            Shape::LabelIn(n) | Shape::LabelOut(n) => (label_width(n), 10.0),
        }
    }

    /// Every port of a fixed symbol, with its position.
    pub fn ports(self) -> &'static [(&'static str, f64, f64)] {
        const TWO_IN: &[(&str, f64, f64)] = &[("A", 0.0, 5.0), ("B", 0.0, 20.0), ("Y", 25.0, 12.5)];
        match self {
            Shape::Mux => &[
                ("A", 0.0, 10.0),
                ("B", 0.0, 30.0),
                ("S", 10.0, 35.0),
                ("Y", 20.0, 20.0),
            ],
            Shape::And => &[("A", 0.0, 5.0), ("B", 0.0, 20.0), ("Y", 30.0, 12.5)],
            Shape::Nand => &[("A", 0.0, 5.0), ("B", 0.0, 20.0), ("Y", 36.0, 12.5)],
            Shape::Or => &[("A", 3.0, 5.0), ("B", 3.0, 20.0), ("Y", 30.0, 12.5)],
            Shape::Nor => &[("A", 3.0, 5.0), ("B", 3.0, 20.0), ("Y", 36.0, 12.5)],
            Shape::Xor => &[("A", 3.0, 5.0), ("B", 3.0, 20.0), ("Y", 33.0, 12.5)],
            Shape::Xnor => &[("A", 3.0, 5.0), ("B", 3.0, 20.0), ("Y", 38.0, 12.5)],
            Shape::Not => &[("A", 0.0, 10.0), ("Y", 25.0, 10.0)],
            Shape::Add | Shape::Sub | Shape::Eq | Shape::Lt | Shape::Le | Shape::Ge | Shape::Gt => {
                TWO_IN
            }
            Shape::Dff => &[
                ("Q", 30.0, 10.0),
                ("CLK", 0.0, 30.0),
                ("C", 0.0, 30.0),
                ("D", 0.0, 10.0),
            ],
            Shape::Dffn => &[
                ("Q", 30.0, 10.0),
                ("CLK", -6.0, 30.0),
                ("C", -6.0, 30.0),
                ("D", 0.0, 10.0),
            ],
            Shape::PinIn => &[("Y", 28.0, 10.0)],
            Shape::PinOut => &[("A", 0.0, 10.0)],
            Shape::Constant => &[("Y", 30.0, 10.0)],
            Shape::LabelIn(n) => match n {
                1 => &[("Y", 18.0, 0.0)],
                2 => &[("Y", 36.0, 0.0)],
                3 => &[("Y", 60.0, 0.0)],
                _ => &[("Y", 96.0, 0.0)],
            },
            Shape::LabelOut(_) => &[("A", 0.0, 0.0)],
            Shape::Generic | Shape::Split | Shape::Join => &[],
        }
    }

    /// A fixed symbol's port position.
    pub fn port(self, pid: &str) -> Option<(f64, f64)> {
        self.ports()
            .iter()
            .find(|(p, _, _)| *p == pid)
            .map(|(_, x, y)| (*x, *y))
    }

    /// Whether the symbol grows with its ports.
    pub fn is_stretchy(self) -> bool {
        matches!(self, Shape::Generic | Shape::Split | Shape::Join)
    }

    /// The symbol's drawing, without its texts, for a block `height` tall
    /// (used by generic blocks, splits and joins). Outlines are `body`
    /// (filled and outlined in the block's colors), inner marks `mark`.
    pub fn draw(self, height: f64) -> String {
        let rect = r#"<rect width="25" height="25" rx="4" class="body"/>"#;
        let line = |x1: f64, y1: f64, x2: f64, y2: f64| {
            format!(r#"<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" class="mark"/>"#)
        };
        let or = r#"<path d="M0,25 L15,25 A15 12.5 0 0 0 15,0 L0,0 Z" class="body"/><path d="M0,0 A30 25 0 0 1 0,25" class="mark"/>"#;
        let xor = r#"<path d="M3,0 A30 25 0 0 1 3,25 A30 25 0 0 0 33,12.5 A30 25 0 0 0 3,0" class="body"/><path d="M0,0 A30 25 0 0 1 0,25" class="mark"/>"#;
        let bubble =
            |cx: f64, cy: f64| format!(r#"<circle cx="{cx}" cy="{cy}" r="3" class="body"/>"#);
        let and = r#"<path d="M0,0 L0,25 L15,25 A15 12.5 0 0 0 15,0 Z" class="body"/>"#;
        let dff = r#"<rect width="30" height="40" class="body"/><path d="M0,35 L5,30 L0,25" class="mark"/>"#;
        let pin = r#"<path d="M0,0 L0,20 L15,20 L30,10 L15,0 Z" class="body"/>"#;
        match self {
            Shape::Mux => r#"<path d="M0,0 L20,10 L20,30 L0,40 Z" class="body"/>"#.into(),
            Shape::And => and.into(),
            Shape::Nand => format!("{and}{}", bubble(34.0, 12.5)),
            Shape::Or => or.into(),
            Shape::Nor => format!("{or}{}", bubble(34.0, 12.5)),
            Shape::Xor => xor.into(),
            Shape::Xnor => format!("{xor}{}", bubble(35.0, 12.5)),
            Shape::Not => format!(
                r#"<path d="M0,0 L0,20 L20,10 Z" class="body"/>{}"#,
                bubble(23.0, 10.0)
            ),
            Shape::Add => format!(
                "{rect}{}{}",
                line(7.5, 12.5, 17.5, 12.5),
                line(12.5, 7.5, 12.5, 17.5)
            ),
            Shape::Sub => format!("{rect}{}", line(7.5, 12.5, 17.5, 12.5)),
            Shape::Eq => format!(
                "{rect}{}{}",
                line(7.5, 10.0, 17.5, 10.0),
                line(7.5, 15.0, 17.5, 15.0)
            ),
            Shape::Lt => format!(
                "{rect}{}{}",
                line(7.5, 12.5, 17.5, 7.5),
                line(7.5, 12.5, 17.5, 17.5)
            ),
            Shape::Le => format!(
                "{rect}{}{}{}",
                line(7.5, 12.5, 17.5, 7.5),
                line(7.5, 12.5, 17.5, 17.5),
                line(7.5, 15.0, 17.5, 20.0)
            ),
            Shape::Gt => format!(
                "{rect}{}{}",
                line(7.5, 7.5, 17.5, 12.5),
                line(7.5, 17.5, 17.5, 12.5)
            ),
            Shape::Ge => format!(
                "{rect}{}{}{}",
                line(7.5, 7.5, 17.5, 12.5),
                line(7.5, 17.5, 17.5, 12.5),
                line(7.5, 20.0, 17.5, 15.0)
            ),
            Shape::Dff => dff.into(),
            Shape::Dffn => format!("{dff}{}", bubble(-3.0, 30.0)),
            Shape::Generic => format!(r#"<rect width="30" height="{height}" class="body"/>"#),
            Shape::PinIn | Shape::PinOut => pin.into(),
            Shape::Constant => r#"<rect width="30" height="20" class="body"/>"#.into(),
            Shape::Split | Shape::Join => {
                format!(r#"<rect width="5" height="{height}" class="bar"/>"#)
            }
            Shape::LabelIn(_) | Shape::LabelOut(_) => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_by_type_and_ports() {
        assert_eq!(
            Shape::for_cell("$add", ["A", "B", "Y"].into_iter()),
            Shape::Add
        );
        assert_eq!(
            Shape::for_cell("$dff", ["CLK", "D", "Q"].into_iter()),
            Shape::Dff
        );
        // A port the symbol has no place for: a box instead.
        assert_eq!(
            Shape::for_cell("$dff", ["CLK", "D", "Q", "EN"].into_iter()),
            Shape::Generic
        );
        assert_eq!(
            Shape::for_cell("$sdff", ["CLK"].into_iter()),
            Shape::Generic
        );
        assert_eq!(
            Shape::for_cell("SB_IO", ["D_IN_0"].into_iter()),
            Shape::Generic
        );
    }

    #[test]
    fn label_sizes() {
        assert_eq!(label_size("0"), 1);
        assert_eq!(label_size("rst"), 2);
        assert_eq!(label_size("sys_clk"), 3);
        assert_eq!(label_size("a_long_reset_name"), 4);
        assert_eq!(Shape::LabelIn(3).port("Y"), Some((60.0, 0.0)));
    }
}
