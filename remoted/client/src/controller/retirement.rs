//! Bounded tombstones protect explicit closure from already queued stream events.
use super::*;

// A connection has at most 8 live streams and 32 queued replies. Keep a larger
// bounded recent set; reset it when selecting a different workspace generation.
const RETIRED_STREAMS: usize = 128;

impl Actor {
    pub(super) fn remember_retired_stream(&mut self, stream: &str) {
        if !self.retired_streams.iter().any(|retired| retired == stream) {
            if self.retired_streams.len() == RETIRED_STREAMS {
                self.retired_streams.pop_front();
            }
            self.retired_streams.push_back(stream.to_owned());
        }
    }

    pub(super) fn retire_stream(&mut self, stream: &str) {
        self.reject_stream_unsent(stream);
        self.remember_retired_stream(stream);
        self.model.panes.retain(|pane| pane.stream != stream);
        self.attachments.remove(stream);
        self.output.remove(stream);
        self.model.interaction.remove_stream(stream);
        self.restoring.remove(stream);
    }

    pub(super) fn is_retired_event(&self, event: &Event) -> bool {
        let stream = match event {
            Event::TerminalOutput { stream, .. }
            | Event::TerminalControl { stream, .. }
            | Event::TerminalExit { stream, .. }
            | Event::TerminalProgramStatus { stream, .. }
            | Event::TerminalUnavailable { stream, .. } => stream,
            _ => return false,
        };
        self.retired_streams.iter().any(|retired| retired == stream)
    }
}
