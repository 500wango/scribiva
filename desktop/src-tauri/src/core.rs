//! Dictation state machine (docs/02 §5, docs/06 §3). Platform independent and
//! side-effect free: callers perform the returned effects (stop capture,
//! cancel the remote job, paste, ...). Every recording gets one job id; any
//! result whose job id is not the active one is ignored, so a late result for
//! a cancelled or superseded job can never be applied.

use serde::Serialize;

/// Externally visible phase (also the UX state name).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Idle,
    Recording,
    Processing,
    /// A user-triggered insert of the last result is running ("insert
    /// anyway" / "insert again"); never entered automatically.
    Injecting,
    /// Text was dispatched (`dispatched_unverified`) or is ready to copy.
    Ready,
    FocusChanged,
    ClipboardOnly,
    SuspectedNoise,
    Quota,
    Error,
    Cancelled,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Idle => "idle",
            Phase::Recording => "recording",
            Phase::Processing => "processing",
            Phase::Injecting => "injecting",
            Phase::Ready => "ready",
            Phase::FocusChanged => "focus_changed",
            Phase::ClipboardOnly => "clipboard_only",
            Phase::SuspectedNoise => "suspected_noise",
            Phase::Quota => "quota",
            Phase::Error => "error",
            Phase::Cancelled => "cancelled",
        }
    }
}

/// Final result of delivering text (docs/02 §4 `InsertOutcome`, desktop subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InsertOutcome {
    /// Committed into the focused text field by the GNOME extension's input
    /// method (Linux). Stronger than a dispatched paste, still not verified
    /// by reading the target back.
    #[cfg_attr(windows, allow(dead_code))]
    Inserted,
    DispatchedUnverified,
    ClipboardOnly,
    FocusChanged,
    SuspectedNoise,
}

impl InsertOutcome {
    /// Outcome enum reported for the north-star metric (docs/11). `notice`
    /// is the delivery reason; `own_window` is a copy-only case of its own.
    pub fn telemetry(self, notice: Option<&str>) -> &'static str {
        match self {
            InsertOutcome::Inserted => "inserted",
            InsertOutcome::DispatchedUnverified => "dispatched_unverified",
            InsertOutcome::FocusChanged => "focus_changed",
            InsertOutcome::SuspectedNoise => "suspected_noise",
            InsertOutcome::ClipboardOnly if notice == Some("own_window") => "own_window",
            InsertOutcome::ClipboardOnly => "clipboard_only",
        }
    }

    fn phase(self) -> Phase {
        match self {
            InsertOutcome::Inserted | InsertOutcome::DispatchedUnverified => Phase::Ready,
            InsertOutcome::ClipboardOnly => Phase::ClipboardOnly,
            InsertOutcome::FocusChanged => Phase::FocusChanged,
            InsertOutcome::SuspectedNoise => Phase::SuspectedNoise,
        }
    }
}

/// Last transcript, memory only (PRD §7.5). Never written to disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LastResult {
    pub job_id: String,
    pub text: String,
    pub raw: String,
    pub warnings: Vec<String>,
    pub suspected_noise: bool,
    pub outcome: Option<InsertOutcome>,
}

/// Failure classes the state machine distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    /// Quota exhausted / plan inactive: show billing, keep nothing to retry.
    Quota,
    /// Token missing or rejected: user must sign in again; audio kept for retry.
    Auth,
    /// The request may or may not have reached the server (timeout, network).
    /// Retry must reuse the same Idempotency-Key so the server can replay.
    Transport,
    /// The server definitively failed or rejected the job; a retry needs a new key.
    Server,
    /// Local problem before upload (microphone, encoding). Nothing retained.
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
    pub class: FailureClass,
    /// Retained audio can be retried.
    pub retryable: bool,
}

/// What the caller must do after `start_recording`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartTicket {
    pub job_id: String,
    /// A previous in-flight job was superseded; cancel it remotely (best effort).
    pub cancel_remote: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopDecision {
    /// Encode and upload with this job id (also the Idempotency-Key).
    Upload { job_id: String },
    /// Too short (< 1 s): dropped without upload.
    TooShort,
    /// Not the active recording; ignore.
    Ignore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CancelEffect {
    StopCapture,
    CancelRemote { job_id: String },
    Nothing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultAction {
    /// Proceed to focus check and (maybe) injection of this text.
    Deliver { text: String },
    /// Result kept in recovery only (suspected hallucination; FR-017).
    KeepOnly,
    /// Stale/cancelled/superseded job: drop silently.
    Ignore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryDecision {
    /// Re-upload the retained audio with this key.
    Upload {
        job_id: String,
    },
    Nothing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    Busy,
}

#[derive(Debug, Clone)]
pub struct Core {
    phase: Phase,
    active_job: Option<String>,
    recording_started_ms: Option<u64>,
    error: Option<ErrorInfo>,
    notice: Option<String>,
    last: Option<LastResult>,
    /// Job id that owns the retained audio (for retry), if any.
    retained_for: Option<String>,
}

impl Default for Core {
    fn default() -> Self {
        Self::new()
    }
}

impl Core {
    pub fn new() -> Self {
        Self {
            phase: Phase::Idle,
            active_job: None,
            recording_started_ms: None,
            error: None,
            notice: None,
            last: None,
            retained_for: None,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }
    pub fn active_job(&self) -> Option<&str> {
        self.active_job.as_deref()
    }
    pub fn error(&self) -> Option<&ErrorInfo> {
        self.error.as_ref()
    }
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }
    pub fn last(&self) -> Option<&LastResult> {
        self.last.as_ref()
    }
    pub fn recording_started_ms(&self) -> Option<u64> {
        self.recording_started_ms
    }
    /// Job whose audio is retained for retry.
    pub fn retained_for(&self) -> Option<&str> {
        self.retained_for.as_deref()
    }

    /// Whether `job_id` is still the job whose result may be applied.
    pub fn is_current(&self, job_id: &str) -> bool {
        self.active_job.as_deref() == Some(job_id)
            && matches!(
                self.phase,
                Phase::Processing | Phase::Recording | Phase::Injecting
            )
    }

    /// Recording, uploading or inserting: no new insert may start.
    pub fn is_busy(&self) -> bool {
        matches!(
            self.phase,
            Phase::Recording | Phase::Processing | Phase::Injecting
        )
    }

    /// The user explicitly asked to insert the last result ("insert
    /// anyway" for suspected noise, "insert again" from recovery). Returns
    /// the text to deliver under the local operation id `op_id`; the caller
    /// takes a fresh focus snapshot. Never called automatically.
    pub fn start_insert(&mut self, op_id: String) -> Option<String> {
        if self.is_busy() {
            return None;
        }
        let text = self.last.as_ref().map(|l| l.text.clone())?;
        if text.trim().is_empty() {
            return None;
        }
        self.phase = Phase::Injecting;
        self.active_job = Some(op_id);
        self.error = None;
        self.notice = None;
        Some(text)
    }

    /// Start a new recording with a fresh job id. A job still processing is
    /// superseded: its result will be ignored and it should be cancelled remotely.
    pub fn start_recording(
        &mut self,
        new_job_id: String,
        now_ms: u64,
    ) -> Result<StartTicket, CoreError> {
        if matches!(self.phase, Phase::Recording | Phase::Injecting) {
            return Err(CoreError::Busy);
        }
        let cancel_remote = if self.phase == Phase::Processing {
            self.active_job.take()
        } else {
            None
        };
        self.phase = Phase::Recording;
        self.active_job = Some(new_job_id.clone());
        self.recording_started_ms = Some(now_ms);
        self.error = None;
        self.notice = None;
        self.retained_for = None;
        Ok(StartTicket {
            job_id: new_job_id,
            cancel_remote,
        })
    }

    /// Recording stopped (by hotkey or the max-duration watchdog).
    pub fn stop_recording(&mut self, job_id: &str, duration_ms: u64) -> StopDecision {
        if self.phase != Phase::Recording || self.active_job.as_deref() != Some(job_id) {
            return StopDecision::Ignore;
        }
        self.recording_started_ms = None;
        if duration_ms < crate::audio_dsp::MIN_RECORDING_MS {
            self.phase = Phase::Idle;
            self.active_job = None;
            self.notice = Some("too_short".into());
            return StopDecision::TooShort;
        }
        self.phase = Phase::Processing;
        self.retained_for = Some(job_id.to_string());
        StopDecision::Upload {
            job_id: job_id.to_string(),
        }
    }

    /// User cancel (Esc / tray / hotkey UI). Never uploads or inserts afterwards.
    pub fn cancel(&mut self) -> CancelEffect {
        match self.phase {
            Phase::Recording => {
                self.phase = Phase::Cancelled;
                self.active_job = None;
                self.recording_started_ms = None;
                self.retained_for = None;
                CancelEffect::StopCapture
            }
            Phase::Processing => {
                self.phase = Phase::Cancelled;
                self.retained_for = None;
                match self.active_job.take() {
                    Some(job_id) => CancelEffect::CancelRemote { job_id },
                    None => CancelEffect::Nothing,
                }
            }
            Phase::Injecting => {
                // Local operation only: nothing to cancel on the server.
                self.phase = Phase::Cancelled;
                self.active_job = None;
                CancelEffect::Nothing
            }
            _ => CancelEffect::Nothing,
        }
    }

    /// A server result arrived for `job_id`.
    pub fn result_arrived(
        &mut self,
        job_id: &str,
        text: String,
        raw: String,
        warnings: Vec<String>,
        suspected_noise: bool,
    ) -> ResultAction {
        if self.phase != Phase::Processing || self.active_job.as_deref() != Some(job_id) {
            return ResultAction::Ignore;
        }
        self.retained_for = None;
        self.last = Some(LastResult {
            job_id: job_id.to_string(),
            text: text.clone(),
            raw,
            warnings,
            suspected_noise,
            outcome: None,
        });
        if suspected_noise {
            self.finish(InsertOutcome::SuspectedNoise);
            return ResultAction::KeepOnly;
        }
        if text.trim().is_empty() {
            // Empty transcript: nothing to insert, keep it visible as "no clear speech".
            self.finish(InsertOutcome::SuspectedNoise);
            return ResultAction::KeepOnly;
        }
        ResultAction::Deliver { text }
    }

    /// Injection/copy finished for `job_id` (a server job, or a manual
    /// insert's operation id). Ignored if cancelled meanwhile.
    pub fn delivered(&mut self, job_id: &str, outcome: InsertOutcome) -> bool {
        if !matches!(self.phase, Phase::Processing | Phase::Injecting)
            || self.active_job.as_deref() != Some(job_id)
        {
            return false;
        }
        self.finish(outcome);
        true
    }

    fn finish(&mut self, outcome: InsertOutcome) {
        if let Some(last) = self.last.as_mut() {
            last.outcome = Some(outcome);
        }
        self.phase = outcome.phase();
        self.active_job = None;
    }

    /// Upload/processing failed for `job_id`.
    pub fn failed(&mut self, job_id: &str, error: ErrorInfo) -> bool {
        if self.active_job.as_deref() != Some(job_id)
            || !matches!(self.phase, Phase::Processing | Phase::Recording)
        {
            return false;
        }
        self.recording_started_ms = None;
        let keep_audio = matches!(
            error.class,
            FailureClass::Transport | FailureClass::Server | FailureClass::Auth
        ) && self.phase == Phase::Processing;
        self.phase = if error.class == FailureClass::Quota {
            Phase::Quota
        } else {
            Phase::Error
        };
        if !keep_audio {
            self.retained_for = None;
        }
        // Keep the failed job id: retry decides whether to reuse it.
        self.error = Some(ErrorInfo {
            retryable: keep_audio && error.retryable,
            ..error
        });
        true
    }

    /// Quota / auth problems detected before recording starts.
    pub fn block_start(&mut self, error: ErrorInfo) {
        if self.is_busy() {
            return;
        }
        self.phase = if error.class == FailureClass::Quota {
            Phase::Quota
        } else {
            Phase::Error
        };
        self.active_job = None;
        self.retained_for = None;
        self.error = Some(error);
    }

    /// Retry the retained audio. Transport failures reuse the same job id
    /// (the server replays the original result instead of charging twice);
    /// definitive server failures need a fresh id (`409 IDEMPOTENCY_CONFLICT`).
    pub fn retry(&mut self, fresh_job_id: String) -> RetryDecision {
        if self.phase != Phase::Error {
            return RetryDecision::Nothing;
        }
        let (Some(err), Some(retained)) = (self.error.as_ref(), self.retained_for.clone()) else {
            return RetryDecision::Nothing;
        };
        if !err.retryable {
            return RetryDecision::Nothing;
        }
        let job_id = match err.class {
            FailureClass::Transport | FailureClass::Auth => retained,
            _ => fresh_job_id,
        };
        self.phase = Phase::Processing;
        self.error = None;
        self.active_job = Some(job_id.clone());
        self.retained_for = Some(job_id.clone());
        RetryDecision::Upload { job_id }
    }

    /// Drop retained audio / pending error and return to idle.
    pub fn discard(&mut self) -> CancelEffect {
        match self.phase {
            Phase::Recording | Phase::Processing | Phase::Injecting => self.cancel(),
            _ => {
                self.phase = Phase::Idle;
                self.error = None;
                self.notice = None;
                self.retained_for = None;
                self.active_job = None;
                CancelEffect::Nothing
            }
        }
    }

    /// Return to idle after the user acknowledged a terminal state.
    pub fn acknowledge(&mut self) {
        if !self.is_busy() {
            self.phase = Phase::Idle;
            self.error = None;
            self.notice = None;
        }
    }

    /// Attach a short machine-readable reason to the current state
    /// (e.g. `terminal`, `elevated`); the UI turns it into text.
    pub fn set_notice(&mut self, notice: Option<String>) {
        self.notice = notice;
    }

    /// Clear the in-memory last result (PRD §7.5).
    pub fn clear_last(&mut self) -> bool {
        self.last.take().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(class: FailureClass, code: &str) -> ErrorInfo {
        ErrorInfo {
            code: code.into(),
            message: code.into(),
            class,
            retryable: true,
        }
    }

    fn recorded(core: &mut Core, id: &str) {
        core.start_recording(id.into(), 0).unwrap();
        assert_eq!(
            core.stop_recording(id, 2_000),
            StopDecision::Upload { job_id: id.into() }
        );
    }

    #[test]
    fn happy_path_dispatches_once() {
        let mut core = Core::new();
        recorded(&mut core, "job-1");
        assert_eq!(core.phase(), Phase::Processing);
        let action = core.result_arrived("job-1", "Hello.".into(), "hello".into(), vec![], false);
        assert_eq!(
            action,
            ResultAction::Deliver {
                text: "Hello.".into()
            }
        );
        assert!(core.delivered("job-1", InsertOutcome::DispatchedUnverified));
        assert_eq!(core.phase(), Phase::Ready);
        // A duplicate delivery for the same job is refused (no double paste).
        assert!(!core.delivered("job-1", InsertOutcome::DispatchedUnverified));
        assert_eq!(
            core.last().unwrap().outcome,
            Some(InsertOutcome::DispatchedUnverified)
        );
    }

    #[test]
    fn cannot_start_twice() {
        let mut core = Core::new();
        core.start_recording("a".into(), 0).unwrap();
        assert_eq!(core.start_recording("b".into(), 1), Err(CoreError::Busy));
    }

    #[test]
    fn short_recording_is_dropped() {
        let mut core = Core::new();
        core.start_recording("job-1".into(), 0).unwrap();
        assert_eq!(core.stop_recording("job-1", 999), StopDecision::TooShort);
        assert_eq!(core.phase(), Phase::Idle);
        assert_eq!(core.notice(), Some("too_short"));
        assert!(core.retained_for().is_none());
    }

    #[test]
    fn cancel_while_recording_never_uploads() {
        let mut core = Core::new();
        core.start_recording("job-1".into(), 0).unwrap();
        assert_eq!(core.cancel(), CancelEffect::StopCapture);
        assert_eq!(core.phase(), Phase::Cancelled);
        // The capture thread reporting the stop afterwards must not upload.
        assert_eq!(core.stop_recording("job-1", 5_000), StopDecision::Ignore);
    }

    #[test]
    fn late_result_after_cancel_is_ignored() {
        let mut core = Core::new();
        recorded(&mut core, "job-1");
        assert_eq!(
            core.cancel(),
            CancelEffect::CancelRemote {
                job_id: "job-1".into()
            }
        );
        let action = core.result_arrived("job-1", "late".into(), "late".into(), vec![], false);
        assert_eq!(action, ResultAction::Ignore);
        assert!(!core.delivered("job-1", InsertOutcome::DispatchedUnverified));
        assert_eq!(core.phase(), Phase::Cancelled);
        assert!(core.last().is_none());
    }

    #[test]
    fn superseded_job_result_is_ignored() {
        let mut core = Core::new();
        recorded(&mut core, "job-1");
        let ticket = core.start_recording("job-2".into(), 10).unwrap();
        assert_eq!(ticket.cancel_remote.as_deref(), Some("job-1"));
        assert_eq!(
            core.result_arrived("job-1", "old".into(), "old".into(), vec![], false),
            ResultAction::Ignore
        );
        assert_eq!(core.phase(), Phase::Recording);
        assert_eq!(core.active_job(), Some("job-2"));
    }

    #[test]
    fn suspected_hallucination_is_never_delivered() {
        let mut core = Core::new();
        recorded(&mut core, "job-1");
        let action = core.result_arrived(
            "job-1",
            "Thanks for watching".into(),
            "Thanks for watching".into(),
            vec!["SUSPECTED_HALLUCINATION".into()],
            true,
        );
        assert_eq!(action, ResultAction::KeepOnly);
        assert_eq!(core.phase(), Phase::SuspectedNoise);
        assert!(core.last().unwrap().suspected_noise);
    }

    #[test]
    fn empty_transcript_is_not_inserted() {
        let mut core = Core::new();
        recorded(&mut core, "job-1");
        assert_eq!(
            core.result_arrived("job-1", "  ".into(), "".into(), vec![], false),
            ResultAction::KeepOnly
        );
    }

    #[test]
    fn focus_change_and_clipboard_only_states() {
        let mut core = Core::new();
        recorded(&mut core, "job-1");
        core.result_arrived("job-1", "t".into(), "t".into(), vec![], false);
        core.delivered("job-1", InsertOutcome::FocusChanged);
        assert_eq!(core.phase(), Phase::FocusChanged);
        recorded(&mut core, "job-2");
        core.result_arrived("job-2", "t".into(), "t".into(), vec![], false);
        core.delivered("job-2", InsertOutcome::ClipboardOnly);
        assert_eq!(core.phase(), Phase::ClipboardOnly);
    }

    #[test]
    fn quota_failure_moves_to_quota_and_drops_audio() {
        let mut core = Core::new();
        recorded(&mut core, "job-1");
        assert!(core.failed("job-1", err(FailureClass::Quota, "QUOTA_EXCEEDED")));
        assert_eq!(core.phase(), Phase::Quota);
        assert!(core.retained_for().is_none());
        assert_eq!(core.retry("job-x".into()), RetryDecision::Nothing);
    }

    #[test]
    fn transport_retry_reuses_idempotency_key() {
        let mut core = Core::new();
        recorded(&mut core, "job-1");
        core.failed("job-1", err(FailureClass::Transport, "NETWORK"));
        assert_eq!(core.phase(), Phase::Error);
        assert_eq!(
            core.retry("job-2".into()),
            RetryDecision::Upload {
                job_id: "job-1".into()
            }
        );
        assert_eq!(core.phase(), Phase::Processing);
    }

    #[test]
    fn server_failure_retry_uses_new_key() {
        let mut core = Core::new();
        recorded(&mut core, "job-1");
        core.failed("job-1", err(FailureClass::Server, "PROVIDER_ERROR"));
        assert_eq!(
            core.retry("job-2".into()),
            RetryDecision::Upload {
                job_id: "job-2".into()
            }
        );
        // The old job's late result can no longer apply.
        assert_eq!(
            core.result_arrived("job-1", "x".into(), "x".into(), vec![], false),
            ResultAction::Ignore
        );
    }

    #[test]
    fn non_retryable_failure_cannot_retry() {
        let mut core = Core::new();
        recorded(&mut core, "job-1");
        let mut e = err(FailureClass::Server, "INVALID_AUDIO");
        e.retryable = false;
        core.failed("job-1", e);
        assert_eq!(core.retry("job-2".into()), RetryDecision::Nothing);
    }

    #[test]
    fn failure_for_stale_job_is_ignored() {
        let mut core = Core::new();
        recorded(&mut core, "job-1");
        core.cancel();
        assert!(!core.failed("job-1", err(FailureClass::Server, "X")));
        assert_eq!(core.phase(), Phase::Cancelled);
    }

    #[test]
    fn discard_and_clear_last() {
        let mut core = Core::new();
        recorded(&mut core, "job-1");
        core.failed("job-1", err(FailureClass::Transport, "NETWORK"));
        assert_eq!(core.discard(), CancelEffect::Nothing);
        assert_eq!(core.phase(), Phase::Idle);
        assert!(core.retained_for().is_none());
        recorded(&mut core, "job-2");
        core.result_arrived("job-2", "t".into(), "r".into(), vec![], false);
        core.delivered("job-2", InsertOutcome::DispatchedUnverified);
        assert!(core.clear_last());
        assert!(core.last().is_none());
        assert!(!core.clear_last());
    }

    #[test]
    fn block_start_shows_quota() {
        let mut core = Core::new();
        core.block_start(err(FailureClass::Quota, "QUOTA_EXCEEDED"));
        assert_eq!(core.phase(), Phase::Quota);
        // A new recording is still possible after upgrading.
        assert!(core.start_recording("job-1".into(), 0).is_ok());
    }

    #[test]
    fn insert_anyway_is_explicit_and_single() {
        let mut core = Core::new();
        // Nothing to insert yet.
        assert_eq!(core.start_insert("op-0".into()), None);
        recorded(&mut core, "job-1");
        core.result_arrived(
            "job-1",
            "Thanks for watching".into(),
            "Thanks for watching".into(),
            vec![],
            true,
        );
        assert_eq!(core.phase(), Phase::SuspectedNoise);
        let text = core.start_insert("op-1".into()).unwrap();
        assert_eq!(text, "Thanks for watching");
        assert_eq!(core.phase(), Phase::Injecting);
        assert!(core.is_current("op-1"));
        // A second insert or a recording cannot start meanwhile.
        assert_eq!(core.start_insert("op-2".into()), None);
        assert_eq!(
            core.start_recording("job-2".into(), 0),
            Err(CoreError::Busy)
        );
        assert!(core.delivered("op-1", InsertOutcome::Inserted));
        assert_eq!(core.phase(), Phase::Ready);
        assert!(!core.delivered("op-1", InsertOutcome::Inserted));
        // Insert again from recovery.
        assert!(core.start_insert("op-3".into()).is_some());
        assert_eq!(core.cancel(), CancelEffect::Nothing);
        assert_eq!(core.phase(), Phase::Cancelled);
        assert!(!core.delivered("op-3", InsertOutcome::Inserted));
    }

    #[test]
    fn insert_refused_while_busy_or_empty() {
        let mut core = Core::new();
        recorded(&mut core, "job-1");
        core.result_arrived("job-1", " ".into(), "".into(), vec![], false);
        assert_eq!(core.start_insert("op".into()), None);
        recorded(&mut core, "job-2");
        assert_eq!(core.phase(), Phase::Processing);
        assert_eq!(core.start_insert("op".into()), None);
    }

    #[test]
    fn telemetry_outcomes_match_server_enum() {
        use InsertOutcome::*;
        assert_eq!(Inserted.telemetry(None), "inserted");
        assert_eq!(
            DispatchedUnverified.telemetry(None),
            "dispatched_unverified"
        );
        assert_eq!(ClipboardOnly.telemetry(Some("terminal")), "clipboard_only");
        assert_eq!(ClipboardOnly.telemetry(Some("own_window")), "own_window");
        assert_eq!(
            FocusChanged.telemetry(Some("focus_changed")),
            "focus_changed"
        );
        assert_eq!(SuspectedNoise.telemetry(None), "suspected_noise");
    }

    #[test]
    fn phase_names_match_ux_states() {
        assert_eq!(Phase::FocusChanged.as_str(), "focus_changed");
        assert_eq!(Phase::SuspectedNoise.as_str(), "suspected_noise");
        assert_eq!(Phase::Injecting.as_str(), "injecting");
        assert_eq!(
            serde_json::to_string(&Phase::ClipboardOnly).unwrap(),
            "\"clipboard_only\""
        );
    }
}
