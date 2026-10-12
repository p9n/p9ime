use std::cell::RefCell;
use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use windows::Win32::Foundation::{
    BOOL, CLASS_E_NOAGGREGATION, E_INVALIDARG, E_POINTER, FALSE, LPARAM, TRUE, WPARAM,
};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_BACK, VK_CAPITAL, VK_CONTROL, VK_ESCAPE, VK_MENU, VK_SHIFT, VK_SPACE,
};
use windows::Win32::UI::TextServices::{
    ITfComposition, ITfCompositionSink, ITfCompositionSink_Impl, ITfContext, ITfContextComposition,
    ITfDocumentMgr, ITfEditSession, ITfEditSession_Impl, ITfInsertAtSelection, ITfKeyEventSink,
    ITfKeyEventSink_Impl, ITfKeystrokeMgr, ITfRange, ITfSource, ITfTextInputProcessor,
    ITfTextInputProcessorEx, ITfTextInputProcessorEx_Impl, ITfTextInputProcessor_Impl,
    ITfThreadMgr, ITfThreadMgrEventSink, ITfThreadMgrEventSink_Impl, TF_AE_NONE, TF_ANCHOR_END,
    TF_ES_READWRITE, TF_ES_SYNC, TF_IAS_QUERYONLY, TF_INVALID_COOKIE, TF_SELECTION,
    TF_SELECTIONSTYLE,
};
use windows_core::{implement, IUnknown, Interface, Result, GUID};

/// Tracks active COM objects and server locks for `DllCanUnloadNow`.
pub static GLOBAL_OBJECT_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Maximum number of letters the test composition accepts.
const MAX_COMPOSITION_LEN: usize = 5;

// ---------------------------------------------------------------------------
// Composition state (test logic)
// ---------------------------------------------------------------------------

/// State of the in-progress composition, shared between the text service and
/// the edit sessions it spawns. TSF text services run in a single-threaded
/// apartment, so `Rc<RefCell<_>>` is sufficient.
#[derive(Default)]
struct CompositionState {
    /// Active TSF composition, if any. `None` when not composing.
    composition: Option<ITfComposition>,
    /// Letters typed so far (at most `MAX_COMPOSITION_LEN`).
    buffer: String,
}

/// What a key press means for the composition.
#[derive(Clone, Copy)]
enum KeyAction {
    /// A–Z: append to the buffer (ignored once full).
    Letter(char),
    /// Backspace while composing: drop the last letter.
    Backspace,
    /// Space while composing: commit the last letter.
    Commit,
    /// Escape while composing: discard the composition.
    Cancel,
}

/// Document edit to perform inside an edit session.
#[derive(Clone, Copy)]
enum CompositionEdit {
    /// Start the composition if needed and set its text to the buffer.
    Update,
    /// Replace the composition with the last buffered letter and end it.
    Commit,
    /// Remove the composition text and end it.
    Cancel,
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

unsafe fn is_key_down(vk: u16) -> bool {
    GetKeyState(vk as i32) < 0
}

unsafe fn is_key_toggled(vk: u16) -> bool {
    GetKeyState(vk as i32) & 1 != 0
}

// ---------------------------------------------------------------------------
// Text service
// ---------------------------------------------------------------------------

struct TextServiceState {
    thread_mgr: Option<ITfThreadMgr>,
    client_id: u32,
    activate_flags: u32,
    thread_mgr_event_sink_cookie: u32,
}

/// Minimal TSF Text Input Processor (TIP) implementation.
#[implement(
    ITfTextInputProcessorEx,
    ITfTextInputProcessor,
    ITfThreadMgrEventSink,
    ITfKeyEventSink,
    ITfCompositionSink
)]
pub struct TextService {
    state: RefCell<TextServiceState>,
    composition: Rc<RefCell<CompositionState>>,
}

impl TextService {
    pub fn new() -> Self {
        GLOBAL_OBJECT_COUNT.fetch_add(1, Ordering::SeqCst);
        Self {
            state: RefCell::new(TextServiceState {
                thread_mgr: None,
                client_id: 0,
                activate_flags: 0,
                thread_mgr_event_sink_cookie: TF_INVALID_COOKIE,
            }),
            composition: Rc::new(RefCell::new(CompositionState::default())),
        }
    }
}

impl Default for TextService {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TextService {
    fn drop(&mut self) {
        GLOBAL_OBJECT_COUNT.fetch_sub(1, Ordering::SeqCst);
    }
}

impl TextService_Impl {
    /// Decide whether (and how) a key press affects the composition. Returning
    /// `None` lets the key pass through to the application untouched.
    fn classify_key(&self, wparam: WPARAM) -> Option<KeyAction> {
        let vk = wparam.0 as u16;
        unsafe {
            // Never swallow shortcuts (Ctrl+C, Alt+Tab, ...).
            if is_key_down(VK_CONTROL.0) || is_key_down(VK_MENU.0) {
                return None;
            }

            let composing = !self.composition.borrow().buffer.is_empty();

            match vk {
                0x41..=0x5A => {
                    let upper = is_key_down(VK_SHIFT.0) ^ is_key_toggled(VK_CAPITAL.0);
                    let c = vk as u8 as char;
                    Some(KeyAction::Letter(if upper {
                        c
                    } else {
                        c.to_ascii_lowercase()
                    }))
                }
                _ if !composing => None,
                v if v == VK_SPACE.0 => Some(KeyAction::Commit),
                v if v == VK_BACK.0 => Some(KeyAction::Backspace),
                v if v == VK_ESCAPE.0 => Some(KeyAction::Cancel),
                _ => None,
            }
        }
    }

    /// Apply a key action: mutate the buffer, then push the change into the
    /// document through a synchronous read/write edit session.
    fn apply_key(&self, context: &ITfContext, action: KeyAction) -> Result<()> {
        // Decide the edit while holding the borrow, then release it before the
        // edit session runs (it re-borrows mutably).
        let edit = {
            let mut st = self.composition.borrow_mut();
            match action {
                KeyAction::Letter(c) => {
                    if st.buffer.chars().count() >= MAX_COMPOSITION_LEN {
                        // Buffer full: swallow the key, nothing to redraw.
                        return Ok(());
                    }
                    st.buffer.push(c);
                    CompositionEdit::Update
                }
                KeyAction::Backspace => {
                    st.buffer.pop();
                    if st.buffer.is_empty() {
                        CompositionEdit::Cancel
                    } else {
                        CompositionEdit::Update
                    }
                }
                KeyAction::Commit => CompositionEdit::Commit,
                KeyAction::Cancel => CompositionEdit::Cancel,
            }
        };

        let client_id = self.state.borrow().client_id;

        unsafe {
            let sink: ITfCompositionSink = self.cast()?;
            let session: ITfEditSession = CompositionEditSession {
                context: context.clone(),
                sink,
                state: Rc::clone(&self.composition),
                edit,
            }
            .into();

            // TF_ES_SYNC: DoEditSession runs before this call returns, so the
            // key is fully processed by the time we report it as eaten.
            context
                .RequestEditSession(client_id, &session, TF_ES_SYNC | TF_ES_READWRITE)?
                .ok()
        }
    }
}

impl ITfTextInputProcessor_Impl for TextService_Impl {
    fn Activate(&self, ptim: Option<&ITfThreadMgr>, tid: u32) -> Result<()> {
        self.ActivateEx(ptim, tid, 0)
    }

    fn Deactivate(&self) -> Result<()> {
        // Drop any composition reference; the app's document is left as-is.
        {
            let mut comp = self.composition.borrow_mut();
            comp.composition = None;
            comp.buffer.clear();
        }

        let mut state = self.state.borrow_mut();

        if let Some(thread_mgr) = state.thread_mgr.take() {
            unsafe {
                // Unadvise KeyEventSink
                if let Ok(keystroke_mgr) = thread_mgr.cast::<ITfKeystrokeMgr>() {
                    let _ = keystroke_mgr.UnadviseKeyEventSink(state.client_id);
                }

                // Unadvise ThreadMgrEventSink
                if state.thread_mgr_event_sink_cookie != TF_INVALID_COOKIE {
                    if let Ok(source) = thread_mgr.cast::<ITfSource>() {
                        let _ = source.UnadviseSink(state.thread_mgr_event_sink_cookie);
                    }
                    state.thread_mgr_event_sink_cookie = TF_INVALID_COOKIE;
                }
            }
        }

        state.client_id = 0;
        state.activate_flags = 0;
        Ok(())
    }
}

impl ITfTextInputProcessorEx_Impl for TextService_Impl {
    fn ActivateEx(&self, ptim: Option<&ITfThreadMgr>, tid: u32, dwflags: u32) -> Result<()> {
        let thread_mgr = ptim.ok_or_else(|| windows_core::Error::from(E_INVALIDARG))?;

        let mut state = self.state.borrow_mut();
        state.thread_mgr = Some(thread_mgr.clone());
        state.client_id = tid;
        state.activate_flags = dwflags;
        state.thread_mgr_event_sink_cookie = TF_INVALID_COOKIE;

        unsafe {
            // Register ITfThreadMgrEventSink
            if let Ok(source) = thread_mgr.cast::<ITfSource>() {
                let sink: ITfThreadMgrEventSink = self.cast()?;
                if let Ok(cookie) = source.AdviseSink(&ITfThreadMgrEventSink::IID, &sink) {
                    state.thread_mgr_event_sink_cookie = cookie;
                }
            }

            // Register ITfKeyEventSink
            if let Ok(keystroke_mgr) = thread_mgr.cast::<ITfKeystrokeMgr>() {
                let key_sink: ITfKeyEventSink = self.cast()?;
                let _ = keystroke_mgr.AdviseKeyEventSink(tid, &key_sink, TRUE);
            }
        }

        Ok(())
    }
}

impl ITfThreadMgrEventSink_Impl for TextService_Impl {
    fn OnInitDocumentMgr(&self, _pdim: Option<&ITfDocumentMgr>) -> Result<()> {
        Ok(())
    }

    fn OnUninitDocumentMgr(&self, _pdim: Option<&ITfDocumentMgr>) -> Result<()> {
        Ok(())
    }

    fn OnSetFocus(
        &self,
        _pdimfocus: Option<&ITfDocumentMgr>,
        _pdimprevfocus: Option<&ITfDocumentMgr>,
    ) -> Result<()> {
        Ok(())
    }

    fn OnPushContext(&self, _pic: Option<&ITfContext>) -> Result<()> {
        Ok(())
    }

    fn OnPopContext(&self, _pic: Option<&ITfContext>) -> Result<()> {
        Ok(())
    }
}

impl ITfKeyEventSink_Impl for TextService_Impl {
    fn OnSetFocus(&self, _fforeground: BOOL) -> Result<()> {
        Ok(())
    }

    fn OnTestKeyDown(
        &self,
        _pic: Option<&ITfContext>,
        wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        // TRUE = we will consume this key in OnKeyDown.
        Ok(BOOL::from(self.classify_key(wparam).is_some()))
    }

    fn OnTestKeyUp(
        &self,
        _pic: Option<&ITfContext>,
        _wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        Ok(FALSE)
    }

    fn OnKeyDown(&self, pic: Option<&ITfContext>, wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        let (Some(action), Some(context)) = (self.classify_key(wparam), pic) else {
            return Ok(FALSE);
        };
        self.apply_key(context, action)?;
        Ok(TRUE)
    }

    fn OnKeyUp(&self, _pic: Option<&ITfContext>, _wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        Ok(FALSE)
    }

    fn OnPreservedKey(&self, _pic: Option<&ITfContext>, _rguid: *const GUID) -> Result<BOOL> {
        Ok(FALSE)
    }
}

impl ITfCompositionSink_Impl for TextService_Impl {
    /// Called when something *other than us* ends the composition (focus change,
    /// the app rejecting it, another TIP, ...). Just forget our state.
    fn OnCompositionTerminated(
        &self,
        _ecwrite: u32,
        _pcomposition: Option<&ITfComposition>,
    ) -> Result<()> {
        // try_borrow_mut: be defensive in case TSF re-enters us mid-edit.
        if let Ok(mut st) = self.composition.try_borrow_mut() {
            st.composition = None;
            st.buffer.clear();
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Edit session
// ---------------------------------------------------------------------------

/// Performs a `CompositionEdit` against the document. Document mutations are
/// only legal inside `DoEditSession`, with the edit cookie TSF hands us.
#[implement(ITfEditSession)]
struct CompositionEditSession {
    context: ITfContext,
    sink: ITfCompositionSink,
    state: Rc<RefCell<CompositionState>>,
    edit: CompositionEdit,
}

impl CompositionEditSession {
    /// Place the caret at the end of `range` so typing continues after the
    /// composition text.
    unsafe fn move_caret_to_end(&self, ec: u32, range: &ITfRange) -> Result<()> {
        let end = range.Clone()?;
        end.Collapse(ec, TF_ANCHOR_END)?;

        let mut selection = TF_SELECTION {
            range: ManuallyDrop::new(Some(end)),
            style: TF_SELECTIONSTYLE {
                ase: TF_AE_NONE,
                fInterimChar: FALSE,
            },
        };
        let result = self
            .context
            .SetSelection(ec, std::slice::from_ref(&selection));
        // TF_SELECTION holds the range as ManuallyDrop; release it ourselves.
        ManuallyDrop::drop(&mut selection.range);
        result
    }

    unsafe fn start_composition(&self, ec: u32) -> Result<ITfComposition> {
        // Query the current selection without inserting anything; the returned
        // range is where the composition will live.
        let insert_at_selection: ITfInsertAtSelection = self.context.cast()?;
        let range = insert_at_selection.InsertTextAtSelection(ec, TF_IAS_QUERYONLY, &[])?;

        let context_composition: ITfContextComposition = self.context.cast()?;
        context_composition.StartComposition(ec, &range, &self.sink)
    }
}

impl ITfEditSession_Impl for CompositionEditSession_Impl {
    fn DoEditSession(&self, ec: u32) -> Result<()> {
        let mut st = self.state.borrow_mut();
        unsafe {
            match self.edit {
                CompositionEdit::Update => {
                    if st.composition.is_none() {
                        st.composition = Some(self.start_composition(ec)?);
                    }
                    let composition = st.composition.as_ref().expect("just set");
                    let range = composition.GetRange()?;
                    range.SetText(ec, 0, &utf16(&st.buffer))?;
                    self.move_caret_to_end(ec, &range)?;
                }
                CompositionEdit::Commit => {
                    if let Some(composition) = st.composition.take() {
                        // Test behaviour: emit only the last typed letter.
                        let last: String = st.buffer.chars().last().into_iter().collect();
                        let range = composition.GetRange()?;
                        range.SetText(ec, 0, &utf16(&last))?;
                        self.move_caret_to_end(ec, &range)?;
                        composition.EndComposition(ec)?;
                    }
                    st.buffer.clear();
                }
                CompositionEdit::Cancel => {
                    if let Some(composition) = st.composition.take() {
                        let range = composition.GetRange()?;
                        range.SetText(ec, 0, &[])?;
                        composition.EndComposition(ec)?;
                    }
                    st.buffer.clear();
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Class factory
// ---------------------------------------------------------------------------

/// COM Class Factory for instantiating `TextService`.
#[implement(IClassFactory)]
pub struct ClassFactory;

impl ClassFactory {
    pub fn new() -> Self {
        GLOBAL_OBJECT_COUNT.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Default for ClassFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ClassFactory {
    fn drop(&mut self) {
        GLOBAL_OBJECT_COUNT.fetch_sub(1, Ordering::SeqCst);
    }
}

impl IClassFactory_Impl for ClassFactory_Impl {
    // Signature is fixed by the `IClassFactory_Impl` trait; raw pointers come from COM callers.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    fn CreateInstance(
        &self,
        punkouter: Option<&IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut c_void,
    ) -> Result<()> {
        if ppvobject.is_null() {
            return Err(windows_core::Error::from(E_POINTER));
        }
        unsafe {
            *ppvobject = std::ptr::null_mut();
        }
        if punkouter.is_some() {
            return Err(windows_core::Error::from(CLASS_E_NOAGGREGATION));
        }
        let service: IUnknown = TextService::new().into();
        unsafe { service.query(riid, ppvobject).ok() }
    }

    fn LockServer(&self, flock: BOOL) -> Result<()> {
        if flock.as_bool() {
            GLOBAL_OBJECT_COUNT.fetch_add(1, Ordering::SeqCst);
        } else {
            GLOBAL_OBJECT_COUNT.fetch_sub(1, Ordering::SeqCst);
        }
        Ok(())
    }
}
