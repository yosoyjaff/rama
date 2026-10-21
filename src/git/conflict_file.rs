//! A file git left in conflict, read into what both sides share and the
//! regions where they disagree, so the merge editor can rebuild it from the
//! lines the user picks.
//!
//! A region runs from a `<<<<<<<` line to a `>>>>>>>` line: our side, then
//! (with `merge.conflictStyle=diff3`) a `|||||||` line and the common
//! ancestor, then `=======` and their side. The ancestor is shown by nothing
//! and taken by nobody, so it is skipped.

/// A stretch of the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictSegment {
    /// Lines both sides agree on.
    Stable(Vec<String>),
    /// A region where they do not.
    Conflict {
        ours: Vec<String>,
        theirs: Vec<String>,
    },
}

/// One side of a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Ours,
    Theirs,
}

/// For each conflict region in order, which of its lines on each side are
/// taken into the result.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConflictTakes {
    regions: Vec<[Vec<bool>; 2]>,
}

impl ConflictTakes {
    /// Whether line `line` of `side` in region `conflict` is taken.
    pub fn is_taken(&self, conflict: usize, side: Side, line: usize) -> bool {
        self.regions
            .get(conflict)
            .and_then(|region| region[Self::side(side)].get(line))
            .copied()
            .unwrap_or(false)
    }

    fn side(side: Side) -> usize {
        match side {
            Side::Ours => 0,
            Side::Theirs => 1,
        }
    }
}

/// Where a line of the file falls.
enum Marker {
    Start,
    Ancestor,
    Split,
    End,
    Text,
}

fn marker(line: &str) -> Marker {
    let has = |prefix: &str| line.starts_with(prefix);
    match () {
        _ if has("<<<<<<<") => Marker::Start,
        _ if has("|||||||") => Marker::Ancestor,
        _ if has("=======") => Marker::Split,
        _ if has(">>>>>>>") => Marker::End,
        _ => Marker::Text,
    }
}

/// Read a conflicted file. A region that never ends (the file was edited
/// half-way through one) keeps what it had, so nothing after it is lost.
pub fn parse_conflict_file(text: &str) -> Vec<ConflictSegment> {
    enum Reading {
        Shared(Vec<String>),
        Ours(Vec<String>),
        Ancestor(Vec<String>),
        Theirs(Vec<String>, Vec<String>),
    }
    let mut segments = Vec::new();
    let mut reading = Reading::Shared(Vec::new());
    for line in text.lines() {
        reading = match (reading, marker(line)) {
            (Reading::Shared(shared), Marker::Start) => {
                if !shared.is_empty() {
                    segments.push(ConflictSegment::Stable(shared));
                }
                Reading::Ours(Vec::new())
            }
            (Reading::Ours(ours), Marker::Ancestor) => Reading::Ancestor(ours),
            (Reading::Ours(ours) | Reading::Ancestor(ours), Marker::Split) => {
                Reading::Theirs(ours, Vec::new())
            }
            (Reading::Theirs(ours, theirs), Marker::End) => {
                segments.push(ConflictSegment::Conflict { ours, theirs });
                Reading::Shared(Vec::new())
            }
            (Reading::Ancestor(ours), _) => Reading::Ancestor(ours),
            (Reading::Shared(mut lines), _) => {
                lines.push(line.to_string());
                Reading::Shared(lines)
            }
            (Reading::Ours(mut lines), _) => {
                lines.push(line.to_string());
                Reading::Ours(lines)
            }
            (Reading::Theirs(ours, mut theirs), _) => {
                theirs.push(line.to_string());
                Reading::Theirs(ours, theirs)
            }
        };
    }
    match reading {
        Reading::Shared(shared) if !shared.is_empty() => {
            segments.push(ConflictSegment::Stable(shared))
        }
        Reading::Shared(_) => {}
        Reading::Ours(ours) | Reading::Ancestor(ours) => segments.push(ConflictSegment::Conflict {
            ours,
            theirs: Vec::new(),
        }),
        Reading::Theirs(ours, theirs) => segments.push(ConflictSegment::Conflict { ours, theirs }),
    }
    segments
}

/// Nothing taken anywhere: how the editor opens.
pub fn empty_takes(segments: &[ConflictSegment]) -> ConflictTakes {
    let regions = segments
        .iter()
        .filter_map(|segment| match segment {
            ConflictSegment::Conflict { ours, theirs } => {
                Some([vec![false; ours.len()], vec![false; theirs.len()]])
            }
            ConflictSegment::Stable(_) => None,
        })
        .collect();
    ConflictTakes { regions }
}

/// The resolved file: the shared lines, and in each region the lines taken
/// from ours and then those taken from theirs. A region with nothing taken
/// leaves nothing.
pub fn output_lines(segments: &[ConflictSegment], takes: &ConflictTakes) -> Vec<String> {
    let mut regions = takes.regions.iter();
    let mut out = Vec::new();
    for segment in segments {
        match segment {
            ConflictSegment::Stable(lines) => out.extend(lines.iter().cloned()),
            ConflictSegment::Conflict { ours, theirs } => {
                let Some([take_ours, take_theirs]) = regions.next() else {
                    continue;
                };
                for (lines, taken) in [(ours, take_ours), (theirs, take_theirs)] {
                    let picked = lines
                        .iter()
                        .zip(taken)
                        .filter(|(_, on)| **on)
                        .map(|(line, _)| line.clone());
                    out.extend(picked);
                }
            }
        }
    }
    out
}

/// Take or leave line `line` of `side` in region `conflict`.
pub fn toggle_line(
    mut takes: ConflictTakes,
    conflict: usize,
    side: Side,
    line: usize,
) -> ConflictTakes {
    if let Some(flag) = takes
        .regions
        .get_mut(conflict)
        .and_then(|region| region[ConflictTakes::side(side)].get_mut(line))
    {
        *flag = !*flag;
    }
    takes
}

/// Take every line of `side`, in every region, or none.
pub fn set_all_taken(mut takes: ConflictTakes, side: Side, taken: bool) -> ConflictTakes {
    for region in &mut takes.regions {
        region[ConflictTakes::side(side)].fill(taken);
    }
    takes
}

/// Whether anything of `side` is taken anywhere.
pub fn side_taken(takes: &ConflictTakes, side: Side) -> bool {
    takes
        .regions
        .iter()
        .any(|region| region[ConflictTakes::side(side)].contains(&true))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "\
head
<<<<<<< HEAD
mine 1
mine 2
||||||| ancestor
old
=======
yours
>>>>>>> topic
middle
<<<<<<< HEAD
a
=======
b
>>>>>>> topic
end
";

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn a_file_splits_into_shared_lines_and_regions() {
        let segments = parse_conflict_file(FILE);
        assert_eq!(
            segments,
            [
                ConflictSegment::Stable(lines(&["head"])),
                ConflictSegment::Conflict {
                    ours: lines(&["mine 1", "mine 2"]),
                    theirs: lines(&["yours"])
                },
                ConflictSegment::Stable(lines(&["middle"])),
                ConflictSegment::Conflict {
                    ours: lines(&["a"]),
                    theirs: lines(&["b"])
                },
                ConflictSegment::Stable(lines(&["end"])),
            ]
        );
    }

    #[test]
    fn a_region_left_open_keeps_its_lines() {
        let segments = parse_conflict_file("x\n<<<<<<< HEAD\nmine\n=======\nyours\n");
        assert_eq!(
            segments[1],
            ConflictSegment::Conflict {
                ours: lines(&["mine"]),
                theirs: lines(&["yours"])
            }
        );
        assert_eq!(
            parse_conflict_file("plain\n"),
            [ConflictSegment::Stable(lines(&["plain"]))]
        );
    }

    #[test]
    fn the_result_is_built_from_what_is_taken() {
        let segments = parse_conflict_file(FILE);
        let none = empty_takes(&segments);
        assert_eq!(output_lines(&segments, &none), ["head", "middle", "end"]);
        assert!(!side_taken(&none, Side::Ours));

        let picked = toggle_line(
            toggle_line(none.clone(), 0, Side::Ours, 1),
            1,
            Side::Theirs,
            0,
        );
        assert_eq!(
            output_lines(&segments, &picked),
            ["head", "mine 2", "middle", "b", "end"]
        );
        // Out of range does nothing.
        assert_eq!(toggle_line(none.clone(), 9, Side::Ours, 0), none);

        let theirs = set_all_taken(none, Side::Theirs, true);
        assert!(side_taken(&theirs, Side::Theirs) && !side_taken(&theirs, Side::Ours));
        assert_eq!(
            output_lines(&segments, &theirs),
            ["head", "yours", "middle", "b", "end"]
        );
        let both = set_all_taken(theirs, Side::Ours, true);
        assert_eq!(
            output_lines(&segments, &both),
            [
                "head", "mine 1", "mine 2", "yours", "middle", "a", "b", "end"
            ]
        );
    }
}
