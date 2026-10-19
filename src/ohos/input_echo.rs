use std::ops::Range;

pub(super) enum InputEcho {
    Unchanged,
    Partial,
    Finished,
}

pub(super) struct PendingInputEcho {
    prefix: String,
    suffix: String,
    inserted: String,
    cursor: usize,
}

impl PendingInputEcho {
    pub(super) fn new(text: &str, selection: Range<usize>, inserted: &str) -> Option<Self> {
        if selection.start != selection.end || inserted.chars().nth(1).is_none() {
            return None;
        }
        let mut cursor = 0;
        let mut byte_offset = 0;
        for character in text.chars() {
            if cursor >= selection.start {
                break;
            }
            cursor += character.len_utf16();
            byte_offset += character.len_utf8();
        }
        if cursor != selection.start {
            return None;
        }
        let (prefix, suffix) = text.split_at(byte_offset);
        Some(Self {
            prefix: prefix.into(),
            suffix: suffix.into(),
            inserted: inserted.into(),
            cursor,
        })
    }

    pub(super) fn observe(&self, text: &str, selection: &Range<usize>) -> InputEcho {
        let Some(inserted) = text
            .strip_prefix(&self.prefix)
            .and_then(|text| text.strip_suffix(&self.suffix))
        else {
            return InputEcho::Finished;
        };
        let cursor = self.cursor + inserted.encode_utf16().count();
        if !self.inserted.starts_with(inserted)
            || inserted.len() >= self.inserted.len()
            || *selection != (cursor..cursor)
        {
            return InputEcho::Finished;
        }
        if inserted.is_empty() {
            InputEcho::Unchanged
        } else {
            // The IME acknowledges one commit using its first selection notification.
            // PTY reads can split that commit, especially while readline redraws a wrap.
            InputEcho::Partial
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapped_pair_waits_for_both_characters() {
        let before = format!("> {}", " ".repeat(116));
        let pending = PendingInputEcho::new(&before, 118..118, "（）").expect("commit");
        assert!(matches!(
            pending.observe(&before, &(118..118)),
            InputEcho::Unchanged
        ));
        assert!(matches!(
            pending.observe(&format!("{before}（"), &(119..119)),
            InputEcho::Partial
        ));
        assert!(matches!(
            pending.observe(&format!("{before}（）"), &(120..120)),
            InputEcho::Finished
        ));
    }

    #[test]
    fn insertion_preserves_the_suffix() {
        let pending = PendingInputEcho::new("$ rest", 2..2, "你好").expect("commit");
        assert!(matches!(
            pending.observe("$ 你rest", &(3..3)),
            InputEcho::Partial
        ));
        assert!(matches!(
            pending.observe("$ 你好rest", &(4..4)),
            InputEcho::Finished
        ));
    }

    #[test]
    fn cursor_offsets_use_utf16() {
        let pending = PendingInputEcho::new("😀 x", 3..3, "😀中").expect("commit");
        assert!(matches!(
            pending.observe("😀 😀x", &(5..5)),
            InputEcho::Partial
        ));
        assert!(matches!(
            pending.observe("😀 😀中x", &(6..6)),
            InputEcho::Finished
        ));
        assert!(matches!(
            pending.observe("😀 😀x", &(4..4)),
            InputEcho::Finished
        ));
    }

    #[test]
    fn transformed_output_and_cursor_movement_release_the_commit() {
        let pending = PendingInputEcho::new("$ ", 2..2, "()").expect("commit");
        for (text, selection) in [("$ (", 2..2), ("$ (other", 8..8), ("new prompt", 10..10)] {
            assert!(matches!(
                pending.observe(text, &selection),
                InputEcho::Finished
            ));
        }
    }

    #[test]
    fn invalid_or_selected_context_does_not_start_a_commit() {
        for selection in [1..1, 4..4, 0..2] {
            assert!(PendingInputEcho::new("😀", selection, "()").is_none());
        }
        assert!(PendingInputEcho::new("", 0..0, "").is_none());
        assert!(PendingInputEcho::new("", 0..0, "😀").is_none());
    }
}
