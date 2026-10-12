use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use windows::Win32::Foundation::{
    BOOL, CLASS_E_NOAGGREGATION, E_INVALIDARG, E_POINTER, FALSE, LPARAM, TRUE, WPARAM,
};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::Win32::UI::TextServices::{
    ITfContext, ITfDocumentMgr, ITfEditSession, ITfEditSession_Impl, ITfInsertAtSelection,
    ITfKeyEventSink, ITfKeyEventSink_Impl, ITfKeystrokeMgr, ITfSource, ITfTextInputProcessor,
    ITfTextInputProcessorEx, ITfTextInputProcessorEx_Impl, ITfTextInputProcessor_Impl,
    ITfThreadMgr, ITfThreadMgrEventSink, ITfThreadMgrEventSink_Impl, TF_IAS_NOQUERY,
    TF_INVALID_COOKIE,
};
use windows_core::{implement, IUnknown, Interface, Result, GUID};

/// Tracks active COM objects and server locks for `DllCanUnloadNow`.
pub static GLOBAL_OBJECT_COUNT: AtomicUsize = AtomicUsize::new(0);

#[derive(Default)]
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
    ITfKeyEventSink
)]
pub struct TextService {
    state: RefCell<TextServiceState>,
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

impl ITfTextInputProcessor_Impl for TextService_Impl {
    fn Activate(&self, ptim: Option<&ITfThreadMgr>, tid: u32) -> Result<()> {
        self.ActivateEx(ptim, tid, 0)
    }

    fn Deactivate(&self) -> Result<()> {
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
        _wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        // Return FALSE to let keystrokes pass through by default in this minimal skeleton.
        // Return TRUE here when you want the IME to intercept and consume a key in OnKeyDown.
        Ok(FALSE)
    }

    fn OnTestKeyUp(
        &self,
        _pic: Option<&ITfContext>,
        _wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        Ok(FALSE)
    }

    fn OnKeyDown(
        &self,
        _pic: Option<&ITfContext>,
        _wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        Ok(FALSE)
    }

    fn OnKeyUp(&self, _pic: Option<&ITfContext>, _wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        Ok(FALSE)
    }

    fn OnPreservedKey(&self, _pic: Option<&ITfContext>, _rguid: *const GUID) -> Result<BOOL> {
        Ok(FALSE)
    }
}

/// Example `ITfEditSession` for inserting text into an `ITfContext` when handling keys.
#[implement(ITfEditSession)]
pub struct InsertTextEditSession {
    context: ITfContext,
    text: Vec<u16>,
}

impl InsertTextEditSession {
    pub fn new(context: ITfContext, text: &str) -> Self {
        Self {
            context,
            text: text.encode_utf16().collect(),
        }
    }
}

impl ITfEditSession_Impl for InsertTextEditSession_Impl {
    fn DoEditSession(&self, ec: u32) -> Result<()> {
        unsafe {
            let insert_at_sel: ITfInsertAtSelection = self.context.cast()?;
            let _ = insert_at_sel.InsertTextAtSelection(ec, TF_IAS_NOQUERY, &self.text)?;
        }
        Ok(())
    }
}

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
