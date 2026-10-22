use serde_json::{Value, json};

use crate::telegram::html;

// A Telegram message described once and rendered two ways: as Rich Message
// blocks (Bot API 10.1+, sendRichMessage) and as classic HTML for sendMessage,
// the fallback when rich messages are rejected.

#[derive(Clone, Debug, PartialEq)]
pub enum Span {
    Plain(String),
    Bold(String),
    Italic(String),
    Code(String),
}

pub fn p(s: impl Into<String>) -> Span {
    Span::Plain(s.into())
}

pub fn b(s: impl Into<String>) -> Span {
    Span::Bold(s.into())
}

pub fn i(s: impl Into<String>) -> Span {
    Span::Italic(s.into())
}

pub fn code(s: impl Into<String>) -> Span {
    Span::Code(s.into())
}

pub type Text = Vec<Span>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TableKind {
    // Two columns: label | value.
    KeyValue,
    // First row holds column titles, first column row titles; numbers right-aligned.
    Grid,
    // Plain rows.
    Rows,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading(Text, u8),
    Para(Text),
    Table(TableKind, Vec<Vec<Text>>),
    List(Vec<Text>),
    Footer(Text),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Msg(pub Vec<Block>);

impl Msg {
    pub fn push(&mut self, b: Block) -> &mut Self {
        self.0.push(b);
        self
    }

    // InputRichMessage
    pub fn rich(&self) -> Value {
        json!({ "blocks": self.0.iter().map(rich_block).collect::<Vec<_>>() })
    }

    pub fn html(&self) -> String {
        let mut out: Vec<String> = Vec::new();
        for (n, block) in self.0.iter().enumerate() {
            out.push(match block {
                // A blank line opens each section after the first.
                Block::Heading(t, _) if n > 0 => format!("\n<b>{}</b>", html(&plain(t))),
                Block::Heading(t, _) => format!("<b>{}</b>", html(&plain(t))),
                Block::Para(t) => html_text(t),
                Block::Table(_, rows) => format!("<pre>{}</pre>", html(&pre_table(rows))),
                Block::List(items) => items.iter().map(|t| format!("• {}", html_text(t))).collect::<Vec<_>>().join("\n"),
                Block::Footer(t) => format!("<i>{}</i>", html(&plain(t))),
            });
        }
        out.join("\n")
    }
}

fn rich_span(s: &Span) -> Value {
    match s {
        Span::Plain(t) => json!(t),
        Span::Bold(t) => json!({ "type": "bold", "text": t }),
        Span::Italic(t) => json!({ "type": "italic", "text": t }),
        Span::Code(t) => json!({ "type": "code", "text": t }),
    }
}

// RichText: a plain string when possible, else an array of pieces.
fn rich_text(t: &Text) -> Value {
    match t.as_slice() {
        [Span::Plain(s)] => json!(s),
        [] => json!(""),
        spans => Value::Array(spans.iter().map(rich_span).collect()),
    }
}

fn paragraph(t: &Text) -> Value {
    json!({ "type": "paragraph", "text": rich_text(t) })
}

fn rich_block(b: &Block) -> Value {
    match b {
        Block::Heading(t, size) => json!({ "type": "heading", "text": rich_text(t), "size": size }),
        Block::Para(t) => paragraph(t),
        Block::Footer(t) => json!({ "type": "footer", "text": rich_text(t) }),
        Block::List(items) => {
            json!({ "type": "list", "items": items.iter().map(|t| json!({ "blocks": [paragraph(t)] })).collect::<Vec<_>>() })
        }
        // State tables (KeyValue, Grid) get borders, a list of rows stays plain;
        // no stripes or shaded label columns, they looked busy in the client.
        Block::Table(kind, rows) => {
            let cells: Vec<Vec<Value>> = rows
                .iter()
                .enumerate()
                .map(|(r, row)| {
                    row.iter()
                        .enumerate()
                        .map(|(c, t)| {
                            let align = if *kind == TableKind::Grid && c > 0 { "right" } else { "left" };
                            let mut cell = json!({ "text": rich_text(t), "align": align });
                            if *kind == TableKind::Grid && r == 0 {
                                cell["is_header"] = json!(true);
                            }
                            cell
                        })
                        .collect()
                })
                .collect();
            let mut table = json!({ "type": "table", "cells": cells });
            if *kind != TableKind::Rows {
                table["is_bordered"] = json!(true);
            }
            table
        }
    }
}

fn plain(t: &Text) -> String {
    t.iter()
        .map(|s| match s {
            Span::Plain(x) | Span::Bold(x) | Span::Italic(x) | Span::Code(x) => x.as_str(),
        })
        .collect()
}

fn html_text(t: &Text) -> String {
    t.iter()
        .map(|s| match s {
            Span::Plain(x) => html(x),
            Span::Bold(x) => format!("<b>{}</b>", html(x)),
            Span::Italic(x) => format!("<i>{}</i>", html(x)),
            Span::Code(x) => format!("<code>{}</code>", html(x)),
        })
        .collect()
}

// Fixed-width table for the HTML fallback.
fn pre_table(rows: &[Vec<Text>]) -> String {
    let cells: Vec<Vec<String>> = rows.iter().map(|r| r.iter().map(plain).collect()).collect();
    let cols = cells.iter().map(|r| r.len()).max().unwrap_or(0);
    let width: Vec<usize> =
        (0..cols).map(|c| cells.iter().filter_map(|r| r.get(c)).map(|s| s.chars().count()).max().unwrap_or(0)).collect();
    cells
        .iter()
        .map(|r| {
            r.iter()
                .enumerate()
                .map(|(c, s)| if c + 1 == r.len() { s.clone() } else { format!("{s:<w$}", w = width[c]) })
                .collect::<Vec<_>>()
                .join("  ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Msg {
        Msg(vec![
            Block::Heading(vec![p("🚨 NOT VOTING")], 2),
            Block::Para(vec![p("Lasted "), b("6s"), p(" <x>")]),
            Block::Table(TableKind::KeyValue, vec![vec![vec![p("Block")], vec![p("115576180")]], vec![vec![p("Peers")], vec![p("214")]]]),
            Block::List(vec![vec![code("/status"), p(": node status")]]),
            Block::Footer(vec![p("monad-mn · 11:27:03")]),
        ])
    }

    #[test]
    fn renders_rich_message_blocks() {
        let r = sample().rich();
        let blocks = r["blocks"].as_array().unwrap();
        assert_eq!(blocks[0], json!({ "type": "heading", "text": "🚨 NOT VOTING", "size": 2 }));
        assert_eq!(blocks[1]["text"], json!(["Lasted ", { "type": "bold", "text": "6s" }, " <x>"]));
        assert_eq!(blocks[2]["type"], "table");
        assert_eq!(blocks[2]["cells"][0][0], json!({ "text": "Block", "align": "left" }));
        assert_eq!(blocks[2]["cells"][0][1], json!({ "text": "115576180", "align": "left" }));
        assert_eq!(blocks[2]["is_bordered"], true);
        assert_eq!(blocks[2].get("is_striped"), None);
        assert_eq!(blocks[3]["items"][0]["blocks"][0]["text"][0], json!({ "type": "code", "text": "/status" }));
        assert_eq!(blocks[4], json!({ "type": "footer", "text": "monad-mn · 11:27:03" }));
    }

    #[test]
    fn renders_the_html_fallback() {
        assert_eq!(
            sample().html(),
            "<b>🚨 NOT VOTING</b>\nLasted <b>6s</b> &lt;x&gt;\n<pre>Block  115576180\nPeers  214</pre>\n• <code>/status</code>: node status\n<i>monad-mn · 11:27:03</i>"
        );
    }
}
