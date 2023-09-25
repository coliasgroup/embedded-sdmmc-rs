use core::str;

use crate::ShortFileName;

const LFN_BUFFER_SIZE: usize = 255 * 4;

#[allow(missing_docs)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct LfnEntry {
    pub is_start: bool,
    pub sequence: u8,
    pub checksum: u8,
    pub buffer: [char; 13],
}

pub(crate) struct LfnVisitor {
    buffer: [u8; LFN_BUFFER_SIZE],
    state: Option<LfnVisitorState>,
}

struct LfnVisitorState {
    buffer_used: usize,
    last_sequence: u8,
    checksum: u8,
}

impl Default for LfnVisitor {
    fn default() -> Self {
        Self {
            buffer: [0; LFN_BUFFER_SIZE],
            state: Default::default(),
        }
    }
}

impl LfnVisitor {
    pub(crate) fn visit(&mut self, entry: &LfnEntry) {
        if entry.is_start {
            if entry.sequence > 20 {
                self.state = None;
            } else {
                let mut state = LfnVisitorState {
                    buffer_used: 0,
                    last_sequence: entry.sequence,
                    checksum: entry.checksum,
                };
                state.extend_buffer(&mut self.buffer, &entry.buffer);
                self.state = Some(state);
            }
        } else if let Some(state) = self.state.as_mut() {
            if state.checksum != entry.checksum || state.last_sequence != entry.sequence + 1 {
                self.state = None;
            } else {
                state.last_sequence -= 1;
                state.extend_buffer(&mut self.buffer, &entry.buffer);
            }
        } else {
            self.state = None;
        }
    }

    pub(crate) fn take(&mut self, short_name: &ShortFileName) -> Option<&str> {
        self.state.take().as_ref().and_then(|state| {
            if state.last_sequence != 1 || state.checksum != short_name.lfn_checksum() {
                None
            } else {
                let active_buffer = &mut self.buffer[..state.buffer_used];
                active_buffer.reverse();
                str::from_utf8(active_buffer)
                    .ok()
                    .map(|s| s.split('\0').next().unwrap())
            }
        })
    }
}

impl LfnVisitorState {
    fn extend_buffer(&mut self, buffer: &mut [u8], extension: &[char]) {
        let remaining_buffer = &mut buffer[self.buffer_used..];
        let mut used = 0;
        for c in extension {
            let s = c.encode_utf8(&mut remaining_buffer[used..]);
            used += s.len();
        }
        remaining_buffer[..used].reverse();
        self.buffer_used += used;
    }
}
