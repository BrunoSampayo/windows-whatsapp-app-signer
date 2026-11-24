use crate::profiles::{Client, has_signature_line, prefix_for_draft};
use std::{
    cell::RefCell,
    mem::ManuallyDrop,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::*,
        Storage::Packaging::Appx::GetPackageFamilyName,
        System::{Com::*, Threading::*, Variant::*},
        UI::{Accessibility::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
    core::{BOOL, BSTR, PWSTR},
};

const TIMEOUT: Duration = Duration::from_secs(6);
const CONFIRM_TIMEOUT: Duration = Duration::from_millis(1200);
const OWN_INPUT: usize = 0x4153_5741;
const COMPOSER_NAMES: &[&str] = &[
    "digite uma mensagem",
    "escreva uma mensagem",
    "escrever mensagem",
    "digite sua mensagem",
    "digitar uma mensagem",
    "type a message",
    "type your message",
];
const SEND_NAMES: &[&str] = &["Enviar", "Enviar mensagem", "Send", "Send message"];
const VOICE_NAMES: &[&str] = &[
    "Mensagem de voz",
    "Gravar mensagem de voz",
    "Voice message",
    "Record voice message",
];

#[derive(Clone, Copy)]
enum Action {
    Enter { extended: bool },
    Click { point: POINT, button: RECT },
}

#[derive(Clone)]
struct Request {
    id: u64,
    window: usize,
    client: Client,
    action: Action,
    name: String,
    expires: Instant,
    hint: Option<POINT>,
    confirm_prefix: Option<String>,
}

type ClickLocation = Arc<Mutex<Option<(usize, POINT)>>>;

#[derive(Default)]
struct Target {
    window: usize,
    client: Client,
    composer_focused: bool,
    buttons: Vec<RECT>,
    voice_slot: Vec<RECT>,
}

enum Event {
    Target(Target),
    Reply { id: u64, decision: Decision },
    Unavailable,
}

enum Decision {
    Pass,
    Insert(String),
    Stop,
}

pub struct Signing {
    keyboard: HHOOK,
    mouse: HHOOK,
    events: Receiver<Event>,
}

struct State {
    requests: Sender<Request>,
    name: Option<String>,
    client: Client,
    web_enabled: Arc<AtomicBool>,
    target: Target,
    foreground: usize,
    whatsapp: bool,
    last_click: ClickLocation,
    pending: Option<Request>,
    next_id: u64,
    enter_down: bool,
    click_down: Option<(usize, RECT)>,
    last_edit: Option<Instant>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

impl Signing {
    pub fn start() -> windows::core::Result<Self> {
        let (requests, inbox) = mpsc::channel();
        let (outbox, events) = mpsc::channel();
        let last_click = ClickLocation::default();
        let worker_hint = Arc::clone(&last_click);
        let web_enabled = Arc::new(AtomicBool::new(false));
        let worker_web_enabled = Arc::clone(&web_enabled);
        std::thread::spawn(move || {
            if with_automation(|automation| {
                monitor(
                    automation,
                    inbox,
                    &outbox,
                    &worker_hint,
                    &worker_web_enabled,
                )
            })
            .is_err()
            {
                let _ = outbox.send(Event::Unavailable);
            }
        });
        let keyboard = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), None, 0)? };
        let mouse = match unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), None, 0) } {
            Ok(mouse) => mouse,
            Err(error) => {
                unsafe {
                    let _ = UnhookWindowsHookEx(keyboard);
                }
                return Err(error);
            }
        };
        STATE.replace(Some(State {
            requests,
            name: None,
            client: Client::Desktop,
            web_enabled,
            target: Target::default(),
            foreground: 0,
            whatsapp: false,
            last_click,
            pending: None,
            next_id: 0,
            enter_down: false,
            click_down: None,
            last_edit: None,
        }));
        Ok(Self {
            keyboard,
            mouse,
            events,
        })
    }

    pub fn set_preferences(&self, name: Option<&str>, client: Client) {
        STATE.with_borrow_mut(|state| {
            if let Some(state) = state {
                state.name = name.map(str::to_owned);
                if state.client != client {
                    state.client = client;
                    state
                        .web_enabled
                        .store(client == Client::Web, Ordering::Relaxed);
                    state.foreground = 0;
                    state.whatsapp = false;
                    state.target = Target::default();
                    state.pending = None;
                    state.last_edit = None;
                }
            }
        });
    }

    pub fn poll(&self) -> bool {
        let mut available = true;
        let mut dispatch = None;
        STATE.with_borrow_mut(|state| {
            let Some(state) = state else { return };
            let window = foreground();
            if window != state.foreground {
                state.foreground = window;
                state.whatsapp =
                    state.client == Client::Desktop && window != 0 && is_whatsapp(window);
                state.last_edit = None;
            }
            for event in self.events.try_iter() {
                match event {
                    Event::Target(mut target) => {
                        if target.client != state.client || target.window != window {
                            continue;
                        }
                        if target.window == state.target.window
                            && target.voice_slot.is_empty()
                            && (target.client == Client::Desktop || target.composer_focused)
                        {
                            target.voice_slot = std::mem::take(&mut state.target.voice_slot);
                        }
                        state.target = target;
                    }
                    Event::Reply { id, decision } => {
                        if state
                            .pending
                            .as_ref()
                            .is_some_and(|request| request.id == id)
                        {
                            dispatch = state.pending.take().map(|request| (request, decision));
                        }
                    }
                    Event::Unavailable => {
                        state.target = Target::default();
                        available = false;
                    }
                }
            }
            if state
                .pending
                .as_ref()
                .is_some_and(|request| Instant::now() >= request.expires)
            {
                dispatch = state.pending.take().map(|request| {
                    let decision =
                        if request.confirm_prefix.is_some() || request.client == Client::Web {
                            Decision::Stop
                        } else {
                            Decision::Pass
                        };
                    (request, decision)
                });
            }
        });
        if let Some((mut request, decision)) = dispatch {
            STATE.with_borrow_mut(|state| {
                if let Some(state) = state {
                    state.last_edit = None;
                }
            });
            match decision {
                Decision::Insert(prefix) if Instant::now() < request.expires => {
                    if valid_destination(&request) && !modifiers_pressed() && insert_prefix(&prefix)
                    {
                        request.confirm_prefix = Some(prefix);
                        request.expires = Instant::now() + CONFIRM_TIMEOUT;
                        STATE.with_borrow_mut(|state| {
                            if let Some(state) = state {
                                state.enqueue(request);
                            }
                        });
                    }
                }
                Decision::Insert(_) | Decision::Pass => send(&request),
                Decision::Stop => {}
            }
        }
        available
    }
}

impl Drop for Signing {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWindowsHookEx(self.keyboard);
            let _ = UnhookWindowsHookEx(self.mouse);
        }
        STATE.replace(None);
    }
}

impl State {
    fn accepts_input(&self, window: usize) -> bool {
        window != 0
            && window == self.foreground
            && match self.client {
                Client::Desktop => self.whatsapp,
                Client::Web => {
                    self.target.client == Client::Web
                        && self.target.window == window
                        && self.target.composer_focused
                }
            }
    }

    fn request(&mut self, window: usize, action: Action) -> bool {
        let Some(name) = self.name.clone() else {
            return false;
        };
        if self.pending.is_some() {
            return true;
        }
        self.next_id = self.next_id.wrapping_add(1);
        let request = Request {
            id: self.next_id,
            window,
            client: self.client,
            action,
            name,
            expires: Instant::now() + TIMEOUT,
            hint: click_hint(&self.last_click, window),
            confirm_prefix: None,
        };
        self.enqueue(request)
    }

    fn enqueue(&mut self, request: Request) -> bool {
        if self.requests.send(request.clone()).is_err() {
            return false;
        }
        self.pending = Some(request);
        true
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let key = unsafe { &*(l.0 as *const KBDLLHOOKSTRUCT) };
        if w.0 as u32 == WM_KEYDOWN
            && key.dwExtraInfo != OWN_INPUT
            && matches!(key.vkCode, 0x20 | 0x30..=0x5a | 0x60..=0x6f | 0xba..=0xe2 | 0xe7)
        {
            STATE.with_borrow_mut(|state| {
                if let Some(state) = state
                    && state.accepts_input(foreground())
                {
                    state.last_edit = Some(Instant::now());
                }
            });
        }
        if key.vkCode == u32::from(VK_RETURN.0)
            && key.dwExtraInfo != OWN_INPUT
            && !key.flags.contains(LLKHF_LOWER_IL_INJECTED)
        {
            let consumed = STATE.with_borrow_mut(|state| {
                let Some(state) = state else { return false };
                match w.0 as u32 {
                    WM_KEYUP | WM_SYSKEYUP => std::mem::take(&mut state.enter_down),
                    WM_KEYDOWN if !modifiers_pressed() => {
                        if state.enter_down {
                            return true;
                        }
                        let window = foreground();
                        if !state.accepts_input(window) {
                            return false;
                        }
                        state.enter_down = state.request(
                            window,
                            Action::Enter {
                                extended: key.flags.contains(LLKHF_EXTENDED),
                            },
                        );
                        state.enter_down
                    }
                    _ => false,
                }
            });
            if consumed {
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, w, l) }
}

unsafe extern "system" fn mouse_hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && matches!(w.0 as u32, WM_LBUTTONDOWN | WM_LBUTTONUP) {
        let mouse = unsafe { &*(l.0 as *const MSLLHOOKSTRUCT) };
        if mouse.dwExtraInfo != OWN_INPUT && mouse.flags & LLMHF_LOWER_IL_INJECTED == 0 {
            let consumed = STATE.with_borrow_mut(|state| {
                let Some(state) = state else { return false };
                let window = foreground();
                if w.0 as u32 == WM_LBUTTONUP {
                    let Some((target, button)) = state.click_down.take() else {
                        return false;
                    };
                    if window == target
                        && contains(button, mouse.pt)
                        && !state.request(
                            window,
                            Action::Click {
                                point: mouse.pt,
                                button,
                            },
                        )
                    {
                        return false;
                    }
                    return true;
                }
                if state.name.is_none() || !state.accepts_input(window) || modifiers_pressed() {
                    return false;
                }
                let target = (state.target.window == window).then_some(&state.target);
                let exact = target.and_then(|target| {
                    target
                        .buttons
                        .iter()
                        .copied()
                        .find(|r| contains(*r, mouse.pt))
                });
                let candidate = exact.or_else(|| {
                    state
                        .last_edit
                        .filter(|time| time.elapsed() <= Duration::from_secs(4))?;
                    // The voice button occupies the send button's position while the draft is empty.
                    target.and_then(|target| {
                        target
                            .voice_slot
                            .iter()
                            .copied()
                            .find(|r| contains(*r, mouse.pt))
                    })
                });
                state.click_down = candidate.map(|rect| (window, rect));
                if state.click_down.is_none()
                    && let Ok(mut hint) = state.last_click.try_lock()
                {
                    *hint = Some((window, mouse.pt));
                }
                state.click_down.is_some()
            });
            if consumed {
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, w, l) }
}

fn foreground() -> usize {
    unsafe { GetForegroundWindow().0 as usize }
}

fn modifiers_pressed() -> bool {
    [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
        .into_iter()
        .any(|key| unsafe { GetAsyncKeyState(i32::from(key.0)) < 0 })
}

fn contains(rect: RECT, point: POINT) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

fn window_handle(value: usize) -> HWND {
    HWND(value as *mut _)
}

fn is_whatsapp(window: usize) -> bool {
    unsafe {
        let mut process_id = 0;
        GetWindowThreadProcessId(window_handle(window), Some(&mut process_id));
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) else {
            return false;
        };
        let mut buffer = [0u16; 128];
        let mut len = buffer.len() as u32;
        let result = GetPackageFamilyName(process, &mut len, Some(PWSTR(buffer.as_mut_ptr())));
        let _ = CloseHandle(process);
        result == ERROR_SUCCESS
            && len != 0
            && String::from_utf16_lossy(&buffer[..len as usize - 1])
                .eq_ignore_ascii_case("5319275A.WhatsAppDesktop_cv1g1gvanyjgm")
    }
}

fn with_automation(
    run: impl FnOnce(&IUIAutomation) -> windows::core::Result<()>,
) -> windows::core::Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let result =
            CoCreateInstance::<_, IUIAutomation2>(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
                .and_then(|automation| {
                    automation.SetConnectionTimeout(350)?;
                    automation.SetTransactionTimeout(350)?;
                    run(&automation)
                });
        CoUninitialize();
        result
    }
}

fn condition(
    automation: &IUIAutomation,
    property: UIA_PROPERTY_ID,
    kind: VARENUM,
    data: VARIANT_0_0_0,
) -> windows::core::Result<IUIAutomationCondition> {
    let mut value = VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                vt: kind,
                Anonymous: data,
                ..Default::default()
            }),
        },
    };
    unsafe {
        let condition = automation.CreatePropertyCondition(property, &value);
        let _ = VariantClear(&mut value);
        condition
    }
}

fn control_conditions(
    automation: &IUIAutomation,
) -> windows::core::Result<(IUIAutomationCondition, IUIAutomationCondition)> {
    let names: Vec<_> = SEND_NAMES
        .iter()
        .chain(VOICE_NAMES)
        .map(|name| {
            condition(
                automation,
                UIA_NamePropertyId,
                VT_BSTR,
                VARIANT_0_0_0 {
                    bstrVal: ManuallyDrop::new(BSTR::from(*name)),
                },
            )
            .map(Some)
        })
        .collect::<windows::core::Result<_>>()?;
    let type_button = condition(
        automation,
        UIA_ControlTypePropertyId,
        VT_I4,
        VARIANT_0_0_0 {
            lVal: UIA_ButtonControlTypeId.0,
        },
    )?;
    let send_or_voice = unsafe {
        let names = automation.CreateOrConditionFromNativeArray(&names)?;
        automation.CreateAndCondition(&type_button, &names)?
    };
    let edit = condition(
        automation,
        UIA_ControlTypePropertyId,
        VT_I4,
        VARIANT_0_0_0 {
            lVal: UIA_EditControlTypeId.0,
        },
    )?;
    let focused = condition(
        automation,
        UIA_HasKeyboardFocusPropertyId,
        VT_BOOL,
        VARIANT_0_0_0 {
            boolVal: VARIANT_BOOL(-1),
        },
    )?;
    let focused_edit = unsafe { automation.CreateAndCondition(&edit, &focused)? };
    Ok((focused_edit, send_or_voice))
}

fn click_hint(last_click: &ClickLocation, window: usize) -> Option<POINT> {
    let hint = *last_click.try_lock().ok()?;
    hint.filter(|(target, _)| *target == window)
        .map(|(_, point)| point)
}

fn monitor(
    automation: &IUIAutomation,
    inbox: Receiver<Request>,
    outbox: &Sender<Event>,
    last_click: &ClickLocation,
    web_enabled: &AtomicBool,
) -> windows::core::Result<()> {
    let (focused_edit, send_or_voice) = control_conditions(automation)?;
    let mut editor = None;
    let mut editor_window = 0;
    let mut editor_client = Client::Desktop;
    loop {
        let request = match inbox.recv_timeout(Duration::from_millis(100)) {
            Ok(request) => Some(request),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let window = foreground();
        let client = if web_enabled.load(Ordering::Relaxed) {
            Client::Web
        } else {
            Client::Desktop
        };
        if window != editor_window || client != editor_client {
            editor = None;
            editor_window = window;
            editor_client = client;
        }
        let event = if let Some(request) = request {
            let current = window == request.window
                && client == request.client
                && Instant::now() < request.expires;
            let can_read = current && client == Client::Desktop && is_whatsapp(window);
            let decision = if request.client == Client::Web {
                if current {
                    web_composer(automation, window).map_or(Decision::Stop, |editor| {
                        decide_web(automation, &request, &editor, &send_or_voice)
                    })
                } else {
                    Decision::Stop
                }
            } else if let Some(prefix) = &request.confirm_prefix {
                if can_read
                    && editor
                        .as_ref()
                        .is_some_and(|editor| confirm_prefix(editor, &request, prefix))
                {
                    Decision::Pass
                } else {
                    Decision::Stop
                }
            } else if can_read {
                editor = editor
                    .filter(|editor| live_composer(editor, window))
                    .or_else(|| focused_composer(automation, window, &focused_edit, request.hint));
                match editor.as_ref().and_then(|editor| {
                    decide(automation, &request, editor, &send_or_voice).unwrap_or_default()
                }) {
                    Some(prefix) => Decision::Insert(prefix),
                    None => Decision::Pass,
                }
            } else {
                Decision::Pass
            };
            Event::Reply {
                id: request.id,
                decision,
            }
        } else {
            if client == Client::Web {
                editor = web_composer(automation, window);
            } else if window != 0 && is_whatsapp(window) {
                editor = editor
                    .filter(|editor| is_composer(editor, window).unwrap_or(false))
                    .or_else(|| {
                        focused_composer(
                            automation,
                            window,
                            &focused_edit,
                            click_hint(last_click, window),
                        )
                    });
            }
            let controls = editor
                .as_ref()
                .and_then(|editor| get_buttons(automation, editor, &send_or_voice).ok())
                .unwrap_or_default();
            let mut buttons = Vec::new();
            let mut voice_slot = Vec::new();
            for (rect, is_send) in controls {
                if is_send {
                    buttons.push(rect);
                } else {
                    voice_slot.push(rect);
                }
            }
            Event::Target(Target {
                window,
                client,
                composer_focused: editor.is_some(),
                buttons,
                voice_slot,
            })
        };
        if outbox.send(event).is_err() {
            break;
        }
    }
    Ok(())
}

fn is_whatsapp_web_url(url: &str) -> bool {
    url.split_once("://").is_some_and(|(scheme, rest)| {
        scheme.eq_ignore_ascii_case("https")
            && rest
                .split(['/', '?', '#'])
                .next()
                .is_some_and(|host| host.eq_ignore_ascii_case("web.whatsapp.com"))
    })
}

fn web_composer(automation: &IUIAutomation, window: usize) -> Option<IUIAutomationElement> {
    unsafe {
        let mut class = [0u16; 32];
        let len = GetClassNameW(window_handle(window), &mut class).max(0) as usize;
        if String::from_utf16_lossy(&class[..len]) != "Chrome_WidgetWin_1" {
            return None;
        }
        let editor = automation.GetFocusedElement().ok()?;
        if !live_composer(&editor, window) {
            return None;
        }
        // Read the loaded document's URL. The address bar can contain uncommitted text.
        let walker = automation.ControlViewWalker().ok()?;
        let mut ancestor = editor.clone();
        for _ in 0..20 {
            ancestor = walker.GetParentElement(&ancestor).ok()?;
            if ancestor.CurrentControlType().ok()? == UIA_DocumentControlTypeId {
                let url = ancestor
                    .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                    .and_then(|pattern| pattern.CurrentValue())
                    .or_else(|_| {
                        ancestor
                            .GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(
                                UIA_LegacyIAccessiblePatternId,
                            )?
                            .CurrentValue()
                    })
                    .ok()?;
                return is_whatsapp_web_url(&url.to_string()).then_some(editor);
            }
        }
        None
    }
}

fn is_composer_name(name: &str) -> bool {
    let name = name.trim().to_lowercase();
    COMPOSER_NAMES.iter().any(|label| {
        name.strip_prefix(label).is_some_and(|tail| {
            tail.is_empty()
                || [" para ", " to ", " for "]
                    .iter()
                    .any(|prefix| tail.starts_with(prefix))
        })
    })
}

fn is_composer(editor: &IUIAutomationElement, window: usize) -> windows::core::Result<bool> {
    unsafe {
        if editor.CurrentControlType()? != UIA_EditControlTypeId
            || !is_composer_name(&editor.CurrentName()?.to_string())
        {
            return Ok(false);
        }
        let rect = editor.CurrentBoundingRectangle()?;
        let center = POINT {
            x: (rect.left + rect.right) / 2,
            y: (rect.top + rect.bottom) / 2,
        };
        let hit = WindowFromPhysicalPoint(center);
        Ok(GetAncestor(hit, GA_ROOT) == window_handle(window))
    }
}

fn live_composer(editor: &IUIAutomationElement, window: usize) -> bool {
    unsafe {
        editor
            .CurrentHasKeyboardFocus()
            .is_ok_and(|focused| focused.as_bool())
            && is_composer(editor, window).unwrap_or(false)
    }
}

unsafe extern "system" fn collect_input_sites(hwnd: HWND, data: LPARAM) -> BOOL {
    let mut name = [0u16; 40];
    let length = unsafe { GetClassNameW(hwnd, &mut name) }.max(0) as usize;
    if String::from_utf16_lossy(&name[..length]) == "InputSiteWindowClass" {
        unsafe {
            (*(data.0 as *mut Vec<HWND>)).push(hwnd);
        }
    }
    BOOL(1)
}

fn focused_composer(
    automation: &IUIAutomation,
    window: usize,
    focused_edit: &IUIAutomationCondition,
    hint: Option<POINT>,
) -> Option<IUIAutomationElement> {
    unsafe {
        if let Ok(editor) = automation.GetFocusedElement()
            && is_composer(&editor, window).unwrap_or(false)
        {
            return Some(editor);
        }
        if let Some(point) = hint
            && GetAncestor(WindowFromPhysicalPoint(point), GA_ROOT) == window_handle(window)
            && let Ok(mut element) = automation.ElementFromPoint(point)
            && let Ok(walker) = automation.ControlViewWalker()
        {
            for _ in 0..3 {
                if live_composer(&element, window) {
                    return Some(element);
                }
                let Ok(parent) = walker.GetParentElement(&element) else {
                    break;
                };
                element = parent;
            }
        }
        // WebView2 can expose its focused content through the XAML input site.
        let mut sites: Vec<HWND> = Vec::new();
        let _ = EnumChildWindows(
            Some(window_handle(window)),
            Some(collect_input_sites),
            LPARAM((&mut sites as *mut Vec<HWND>) as isize),
        );
        for site in sites {
            let Ok(elements) = automation
                .ElementFromHandle(site)
                .and_then(|root| root.FindAll(TreeScope_Descendants, focused_edit))
            else {
                continue;
            };
            for index in 0..elements.Length().unwrap_or(0) {
                if let Ok(editor) = elements.GetElement(index)
                    && is_composer(&editor, window).unwrap_or(false)
                {
                    return Some(editor);
                }
            }
        }
        None
    }
}

fn get_buttons(
    automation: &IUIAutomation,
    editor: &IUIAutomationElement,
    send_or_voice: &IUIAutomationCondition,
) -> windows::core::Result<Vec<(RECT, bool)>> {
    unsafe {
        let walker = automation.ControlViewWalker()?;
        let mut parent = editor.clone();
        let mut buttons = Vec::new();
        for _ in 0..5 {
            parent = walker.GetParentElement(&parent)?;
            if [UIA_DocumentControlTypeId, UIA_WindowControlTypeId]
                .contains(&parent.CurrentControlType()?)
            {
                break;
            }
            let elements = parent.FindAll(TreeScope_Descendants, send_or_voice)?;
            for index in 0..elements.Length()? {
                let element = elements.GetElement(index)?;
                if element.CurrentIsOffscreen()?.as_bool() || !element.CurrentIsEnabled()?.as_bool()
                {
                    continue;
                }
                let r = element.CurrentBoundingRectangle()?;
                if r.right > r.left && r.bottom > r.top {
                    let name = element.CurrentName()?;
                    buttons.push((r, SEND_NAMES.contains(&name.to_string().as_str())));
                }
            }
            if !buttons.is_empty() {
                break;
            }
        }
        Ok(buttons)
    }
}

fn decide_web(
    automation: &IUIAutomation,
    request: &Request,
    editor: &IUIAutomationElement,
    send_or_voice: &IUIAutomationCondition,
) -> Decision {
    if let Action::Click { point, .. } = request.action {
        let button = get_buttons(automation, editor, send_or_voice)
            .unwrap_or_default()
            .into_iter()
            .find(|(rect, _)| contains(*rect, point));
        match button {
            None => return Decision::Stop,
            Some((_, false)) => return Decision::Pass,
            Some((_, true)) => {}
        }
    }
    if let Some(prefix) = &request.confirm_prefix {
        return if confirm_prefix(editor, request, prefix) {
            Decision::Pass
        } else {
            Decision::Stop
        };
    }
    draft_text(editor)
        .ok()
        .and_then(|draft| prefix_for_draft(&request.name, &draft.to_string()))
        .map_or(Decision::Pass, Decision::Insert)
}

fn decide(
    automation: &IUIAutomation,
    request: &Request,
    editor: &IUIAutomationElement,
    send_or_voice: &IUIAutomationCondition,
) -> windows::core::Result<Option<String>> {
    if let Action::Click { point, .. } = request.action
        && !get_buttons(automation, editor, send_or_voice)?
            .into_iter()
            .any(|(button, is_send)| is_send && contains(button, point))
    {
        return Ok(None);
    }
    let draft = draft_text(editor)?;
    if !unsafe { editor.CurrentHasKeyboardFocus() }?.as_bool() {
        return Ok(None);
    }
    Ok(prefix_for_draft(&request.name, &draft.to_string()))
}

fn draft_text(editor: &IUIAutomationElement) -> windows::core::Result<BSTR> {
    unsafe {
        if let Ok(pattern) =
            editor.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
        {
            pattern.CurrentValue()
        } else {
            let pattern =
                editor.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)?;
            pattern.DocumentRange()?.GetText(-1)
        }
    }
}

fn confirm_prefix(editor: &IUIAutomationElement, request: &Request, prefix: &str) -> bool {
    while Instant::now() < request.expires && foreground() == request.window {
        if draft_text(editor).is_ok_and(|draft| has_signature_line(&draft.to_string(), prefix))
            && unsafe { editor.CurrentHasKeyboardFocus() }.is_ok_and(|focused| focused.as_bool())
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

fn keyboard(vk: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                dwExtraInfo: OWN_INPUT,
                ..Default::default()
            },
        },
    }
}

fn mouse(flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dwFlags: flags,
                dwExtraInfo: OWN_INPUT,
                ..Default::default()
            },
        },
    }
}

fn valid_destination(request: &Request) -> bool {
    if foreground() != request.window {
        return false;
    }
    if let Action::Click { button, point } = request.action {
        let mut cursor = POINT::default();
        if unsafe { GetPhysicalCursorPos(&mut cursor) }.is_err()
            || !contains(button, cursor)
            || (cursor.x - point.x).abs() > 5
            || (cursor.y - point.y).abs() > 5
        {
            return false;
        }
    }
    true
}

fn insert_prefix(prefix: &str) -> bool {
    let mut inputs = Vec::from([
        keyboard(VK_CONTROL, 0, KEYBD_EVENT_FLAGS(0)),
        keyboard(VK_HOME, 0, KEYEVENTF_EXTENDEDKEY),
        keyboard(VK_HOME, 0, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP),
        keyboard(VK_CONTROL, 0, KEYEVENTF_KEYUP),
    ]);
    for code in prefix.encode_utf16() {
        inputs.push(keyboard(VIRTUAL_KEY(0), code, KEYEVENTF_UNICODE));
        inputs.push(keyboard(
            VIRTUAL_KEY(0),
            code,
            KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
        ));
    }
    // Shift+Enter inserts a line break in the WhatsApp composer without sending the draft.
    inputs.extend([
        keyboard(VK_SHIFT, 0, KEYBD_EVENT_FLAGS(0)),
        keyboard(VK_RETURN, 0, KEYBD_EVENT_FLAGS(0)),
        keyboard(VK_RETURN, 0, KEYEVENTF_KEYUP),
        keyboard(VK_SHIFT, 0, KEYEVENTF_KEYUP),
    ]);
    unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) as usize == inputs.len() }
}

fn send(request: &Request) {
    if !valid_destination(request) {
        return;
    }
    let mut inputs = Vec::new();
    match request.action {
        Action::Enter { extended } => {
            let flags = if extended {
                KEYEVENTF_EXTENDEDKEY
            } else {
                KEYBD_EVENT_FLAGS(0)
            };
            inputs.push(keyboard(VK_RETURN, 0, flags));
            inputs.push(keyboard(VK_RETURN, 0, flags | KEYEVENTF_KEYUP));
        }
        Action::Click { .. } => {
            inputs.push(mouse(MOUSEEVENTF_LEFTDOWN));
            inputs.push(mouse(MOUSEEVENTF_LEFTUP));
        }
    }
    unsafe {
        SendInput(&inputs, size_of::<INPUT>() as i32);
    }
}

#[cfg(test)]
mod tests {
    use super::{is_composer_name, is_whatsapp_web_url};

    #[test]
    fn identifies_the_loaded_whatsapp_web_origin() {
        assert!(is_whatsapp_web_url("https://web.whatsapp.com/"));
        assert!(is_whatsapp_web_url("https://WEB.WHATSAPP.COM/?a=b"));
        for other in [
            "http://web.whatsapp.com/",
            "https://web.whatsapp.com.example.org/",
            "https://web.whatsapp.com@example.org/",
            "https://example.org/?page=https://web.whatsapp.com/",
        ] {
            assert!(!is_whatsapp_web_url(other));
        }
    }

    #[test]
    fn recognizes_conversation_names_in_the_composer_label() {
        assert!(is_composer_name("Digite uma mensagem para o grupo Plantão"));
        assert!(is_composer_name("Type a message"));
        assert!(is_composer_name("Type a message to Alex"));
        assert!(!is_composer_name("Pesquisar mensagens"));
    }
}
