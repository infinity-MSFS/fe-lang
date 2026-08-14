#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct UnitId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub unit: UnitId,
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(unit: UnitId, start: usize, end: usize) -> Span {
        Span {
            unit,
            start: start as u32,
            end: end as u32,
        }
    }

    pub fn to(self, other: Span) -> Span {
        Span {
            unit: self.unit,
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    pub fn contains(&self, offset: u32) -> bool {
        self.start <= offset && offset <= self.end
    }

    pub fn len(&self) -> usize {
        (self.end - self.start) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }
}

#[derive(Debug, Clone)]
pub struct SourceUnit {
    name: String,
    text: String,
}

impl SourceUnit {
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> SourceUnit {
        SourceUnit {
            name: name.into(),
            text: text.into(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Debug, Clone, Default)]
pub struct SourceMap<'a> {
    units: Vec<&'a SourceUnit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    pub line: u32,
    pub column: u32,
}

impl<'a> SourceMap<'a> {
    pub fn new(units: &'a [SourceUnit]) -> SourceMap<'a> {
        SourceMap {
            units: units.iter().collect(),
        }
    }

    pub fn unit(&self, id: UnitId) -> Option<&'a SourceUnit> {
        self.units.get(id.0 as usize).copied()
    }

    pub fn name(&self, id: UnitId) -> &str {
        self.unit(id).map(|u| u.name()).unwrap_or("<unknown>")
    }

    pub fn snippet(&self, span: Span) -> &'a str {
        self.unit(span.unit)
            .and_then(|u| u.text().get(span.start as usize..span.end as usize))
            .unwrap_or("")
    }

    pub fn location(&self, span: Span) -> Location {
        let text = match self.unit(span.unit) {
            Some(u) => u.text(),
            None => return Location { line: 1, column: 1 },
        };
        let offset = (span.start as usize).min(text.len());
        let mut line = 1u32;
        let mut line_start = 0usize;
        for (i, b) in text.as_bytes().iter().enumerate().take(offset) {
            if *b == b'\n' {
                line += 1;
                line_start = i + 1;
            }
        }
        let column = text
            .get(line_start..offset)
            .map(|s| s.chars().count() as u32)
            .unwrap_or(0)
            + 1;
        Location { line, column }
    }

    pub fn line_text(&self, span: Span) -> &'a str {
        let text = match self.unit(span.unit) {
            Some(u) => u.text(),
            None => return "",
        };
        let offset = (span.start as usize).min(text.len());
        let start = text[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let end = text[offset..]
            .find('\n')
            .map(|i| offset + i)
            .unwrap_or(text.len());
        &text[start..end]
    }
}
