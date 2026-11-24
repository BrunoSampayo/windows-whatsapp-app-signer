use crate::{
    profiles::{Client, MAX_NAME, MAX_PROFILES, Profiles},
    signing::Signing,
};
use std::{
    cell::{Cell, RefCell},
    error::Error,
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::{COLOR_WINDOW, DEFAULT_GUI_FONT, GetStockObject, HBRUSH},
        System::{LibraryLoader::GetModuleHandleW, Threading::CreateMutexW},
        UI::{
            Controls::EM_SETLIMITTEXT,
            HiDpi::GetDpiForSystem,
            Input::KeyboardAndMouse::{
                MOD_ALT, MOD_NOREPEAT, RegisterHotKey, SetFocus, UnregisterHotKey, VK_W,
            },
            Shell::*,
            WindowsAndMessaging::*,
        },
    },
    core::{PCWSTR, w},
};

const CLASS: PCWSTR = w!("AssinadorWhatsApp.Configuracao");
const TITLE: PCWSTR = w!("Assinador para WhatsApp");
const TRAY: u32 = WM_APP + 1;
const SHOW: u32 = WM_APP + 2;
const MANAGE_HOTKEY: i32 = 1;
const LIST: usize = 100;
const ADD: usize = 101;
const RENAME: usize = 102;
const REMOVE: usize = 103;
const SELECT: usize = 104;
const MANAGE: usize = 105;
const PAUSE: usize = 106;
const EXIT: usize = 107;
const CLIENT: usize = 108;
const PROFILE_BASE: usize = 200;
const ICON_FILE: &[u8] = include_bytes!("../assets/caneca.ico");
#[repr(align(4))]
struct AlignedIcon([u8; ICON_FILE.len()]);
static CANECA: AlignedIcon = AlignedIcon(*include_bytes!("../assets/caneca.ico"));

struct App {
    hwnd: HWND,
    edit: HWND,
    list: HWND,
    status: HWND,
    client: HWND,
    profiles: Profiles,
    signing: Signing,
    automation_available: bool,
    icon: HICON,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
}

pub fn run() -> Result<(), Box<dyn Error>> {
    let background = std::env::args_os().any(|argument| argument == "--background");
    unsafe {
        let _ = SetProcessDPIAware();
        let mutex = CreateMutexW(None, false, w!("Local\\AssinadorWhatsApp.Instancia"))?;
        let already_running = GetLastError() == ERROR_ALREADY_EXISTS;
        let result = if already_running {
            if !background && let Ok(window) = FindWindowW(CLASS, None) {
                let _ = PostMessageW(Some(window), SHOW, WPARAM(0), LPARAM(0));
            }
            Ok(())
        } else {
            run_window(background)
        };
        let _ = CloseHandle(mutex);
        result
    }
}

fn run_window(background: bool) -> Result<(), Box<dyn Error>> {
    let profiles = Profiles::load()?;
    unsafe {
        let instance = HINSTANCE(GetModuleHandleW(None)?.0);
        let data_offset = u32::from_le_bytes(CANECA.0[18..22].try_into()?) as usize;
        let icon = CreateIconFromResourceEx(
            &CANECA.0[data_offset..],
            true,
            0x0003_0000,
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
            LR_DEFAULTCOLOR,
        )?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hIcon: icon,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as usize as *mut _),
            lpszClassName: CLASS,
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(windows::core::Error::from_thread().into());
        }
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS,
            TITLE,
            WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            scale(490),
            scale(415),
            None,
            None,
            Some(instance),
            None,
        )?;
        let status = control(hwnd, w!("STATIC"), w!(""), 0, WS_VISIBLE, [20, 20, 430, 24])?;
        control(
            hwnd,
            w!("STATIC"),
            w!("Versão do WhatsApp"),
            0,
            WS_VISIBLE,
            [20, 53, 140, 23],
        )?;
        let client = control(
            hwnd,
            w!("COMBOBOX"),
            w!(""),
            CLIENT,
            WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST as u32),
            [162, 49, 288, 110],
        )?;
        for label in [w!("Aplicativo do Windows"), w!("Navegador (WhatsApp Web)")] {
            SendMessageW(
                client,
                CB_ADDSTRING,
                None,
                Some(LPARAM(label.as_ptr() as isize)),
            );
        }
        let list = control(
            hwnd,
            w!("LISTBOX"),
            w!(""),
            LIST,
            WS_VISIBLE | WS_TABSTOP | WS_BORDER | WS_VSCROLL | WINDOW_STYLE(LBS_NOTIFY as u32),
            [20, 92, 430, 128],
        )?;
        control(
            hwnd,
            w!("STATIC"),
            w!("Nome"),
            0,
            WS_VISIBLE,
            [20, 236, 58, 23],
        )?;
        let edit = control(
            hwnd,
            w!("EDIT"),
            w!(""),
            0,
            WS_VISIBLE | WS_TABSTOP | WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            [82, 233, 368, 26],
        )?;
        SendMessageW(edit, EM_SETLIMITTEXT, Some(WPARAM(MAX_NAME)), None);
        for (label, id, x, width) in [
            (w!("Adicionar"), ADD, 20, 95),
            (w!("Renomear"), RENAME, 125, 95),
            (w!("Remover"), REMOVE, 230, 95),
            (w!("Usar este nome"), SELECT, 335, 115),
        ] {
            control(
                hwnd,
                w!("BUTTON"),
                label,
                id,
                WS_VISIBLE | WS_TABSTOP,
                [x, 278, width, 29],
            )?;
        }
        let hotkey = RegisterHotKey(
            Some(hwnd),
            MANAGE_HOTKEY,
            MOD_ALT | MOD_NOREPEAT,
            u32::from(VK_W.0),
        );
        let footer = if hotkey.is_ok() {
            w!("Alt+W abre esta janela. Fechar mantém o Assinador na bandeja.")
        } else {
            w!("Alt+W indisponível. Abra pelo ícone na bandeja.")
        };
        control(
            hwnd,
            w!("STATIC"),
            footer,
            0,
            WS_VISIBLE,
            [20, 322, 430, 22],
        )?;
        let version = wide(concat!("Versão ", env!("CARGO_PKG_VERSION")));
        control(
            hwnd,
            w!("STATIC"),
            PCWSTR(version.as_ptr()),
            0,
            WS_VISIBLE,
            [20, 350, 430, 22],
        )?;
        let signing = Signing::start()?;
        let app = App {
            hwnd,
            edit,
            list,
            status,
            client,
            profiles,
            signing,
            icon,
            automation_available: true,
        };
        TASKBAR_CREATED.set(RegisterWindowMessageW(w!("TaskbarCreated")));
        app.refresh(Some(app.profiles.active));
        let tray_added = app.notify(NIM_ADD);
        APP.replace(Some(app));
        if SetTimer(Some(hwnd), 1, 25, None) == 0 {
            let _ = DestroyWindow(hwnd);
            return Err(windows::core::Error::from_thread().into());
        }
        if !background || !tray_added {
            show(hwnd);
        }
        let mut msg = MSG::default();
        loop {
            let result = GetMessageW(&mut msg, None, 0, 0).0;
            if result <= 0 {
                APP.replace(None);
                let _ = DestroyIcon(icon);
                return if result < 0 {
                    Err(windows::core::Error::from_thread().into())
                } else {
                    Ok(())
                };
            }
            if !IsWindowVisible(hwnd).as_bool() || !IsDialogMessageW(hwnd, &msg).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

fn scale(value: i32) -> i32 {
    value * unsafe { GetDpiForSystem() } as i32 / 96
}

fn control(
    parent: HWND,
    class: PCWSTR,
    label: PCWSTR,
    id: usize,
    style: WINDOW_STYLE,
    rect: [i32; 4],
) -> windows::core::Result<HWND> {
    unsafe {
        let [x, y, width, height] = rect.map(scale);
        let control = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            label,
            WS_CHILD | style,
            x,
            y,
            width,
            height,
            Some(parent),
            Some(HMENU(id as *mut _)),
            None,
            None,
        )?;
        let font = GetStockObject(DEFAULT_GUI_FONT);
        SendMessageW(
            control,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );
        Ok(control)
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

fn show(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_RESTORE);
        let _ = SetForegroundWindow(hwnd);
        APP.with_borrow(|app| {
            if let Some(app) = app {
                let _ = SetFocus(Some(app.edit));
            }
        });
    }
}

pub fn message(text: &str) {
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(wide(text).as_ptr()),
            TITLE,
            MB_OK | MB_ICONERROR,
        );
    }
}

impl App {
    fn status_text(&self) -> String {
        if !self.automation_available {
            return "Assinatura indisponível. Reinicie o Assinador.".into();
        }
        if self.profiles.paused {
            return "Assinatura pausada".into();
        }
        self.profiles.signing_name().map_or_else(
            || "Cadastre um nome para começar.".into(),
            |name| format!("Operador ativo: {name}"),
        )
    }

    fn notify(&self, action: NOTIFY_ICON_MESSAGE) -> bool {
        let mut icon = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: TRAY,
            hIcon: self.icon,
            ..Default::default()
        };
        let text = format!("Assinador para WhatsApp\n{}", self.status_text());
        for (slot, code) in icon.szTip[..127].iter_mut().zip(text.encode_utf16()) {
            *slot = code;
        }
        unsafe { Shell_NotifyIconW(action, &icon).as_bool() }
    }

    fn refresh(&self, selection: Option<usize>) {
        self.signing
            .set_preferences(self.profiles.signing_name(), self.profiles.client);
        self.refresh_client();
        unsafe {
            let _ = SetWindowTextW(self.status, PCWSTR(wide(&self.status_text()).as_ptr()));
            SendMessageW(self.list, LB_RESETCONTENT, None, None);
            for (index, name) in self.profiles.names.iter().enumerate() {
                let text = if index == self.profiles.active {
                    wide(&format!("{name} (atual)"))
                } else {
                    wide(name)
                };
                SendMessageW(
                    self.list,
                    LB_ADDSTRING,
                    None,
                    Some(LPARAM(text.as_ptr() as isize)),
                );
            }
            if let Some(index) = selection.filter(|&index| index < self.profiles.names.len()) {
                SendMessageW(self.list, LB_SETCURSEL, Some(WPARAM(index)), None);
            }
        }
        self.notify(NIM_MODIFY);
    }

    fn refresh_client(&self) {
        unsafe {
            SendMessageW(
                self.client,
                CB_SETCURSEL,
                Some(WPARAM(usize::from(self.profiles.client == Client::Web))),
                None,
            );
        }
    }

    fn selected(&self) -> Option<usize> {
        let index = unsafe { SendMessageW(self.list, LB_GETCURSEL, None, None).0 };
        usize::try_from(index)
            .ok()
            .filter(|&index| index < self.profiles.names.len())
    }

    fn edit_name(&self) -> String {
        let mut buffer = [0; MAX_NAME + 1];
        let len = unsafe { GetWindowTextW(self.edit, &mut buffer) } as usize;
        String::from_utf16_lossy(&buffer[..len]).trim().to_owned()
    }

    fn command(&mut self, command: usize) -> Result<(), String> {
        let mut next = self.profiles.clone();
        let mut selection = self.selected();
        let needs_selection = || selection.ok_or_else(|| "Selecione um nome na lista.".to_owned());
        match command {
            ADD => {
                if next.names.len() >= MAX_PROFILES {
                    return Err("O limite é de 100 nomes.".into());
                }
                let name = self.edit_name();
                next.validate_name(&name, None)?;
                next.names.push(name);
                selection = Some(next.names.len() - 1);
            }
            RENAME => {
                let index = needs_selection()?;
                let name = self.edit_name();
                next.validate_name(&name, Some(index))?;
                next.names[index] = name;
            }
            REMOVE => {
                let index = needs_selection()?;
                next.names.remove(index);
                next.active = (next.active - usize::from(index < next.active))
                    .min(next.names.len().saturating_sub(1));
                selection = Some(next.active);
            }
            SELECT => next.active = needs_selection()?,
            CLIENT => {
                next.client = match unsafe { SendMessageW(self.client, CB_GETCURSEL, None, None).0 }
                {
                    0 => Client::Desktop,
                    1 => Client::Web,
                    _ => return Ok(()),
                };
                if next.client == self.profiles.client {
                    return Ok(());
                }
            }
            PAUSE => next.paused = !next.paused,
            id if (PROFILE_BASE..PROFILE_BASE + next.names.len()).contains(&id) => {
                next.active = id - PROFILE_BASE;
                next.paused = false;
                selection = Some(next.active);
            }
            _ => return Ok(()),
        }
        if let Err(error) = next.save() {
            self.refresh_client();
            return Err(format!(
                "Não foi possível salvar as preferências.\n\n{error}"
            ));
        }
        self.profiles = next;
        if matches!(command, ADD | RENAME) {
            unsafe {
                let _ = SetWindowTextW(self.edit, w!(""));
            }
        }
        self.refresh(selection);
        Ok(())
    }
}

fn tray_menu(hwnd: HWND) -> windows::core::Result<()> {
    unsafe {
        let menu = CreatePopupMenu()?;
        let result = APP.with_borrow(|app| -> windows::core::Result<()> {
            let Some(app) = app else { return Ok(()) };
            if !app.profiles.names.is_empty() {
                let people = CreatePopupMenu()?;
                if let Err(error) =
                    AppendMenuW(menu, MF_POPUP, people.0 as usize, w!("Trocar operador"))
                {
                    let _ = DestroyMenu(people);
                    return Err(error);
                }
                for (index, name) in app.profiles.names.iter().enumerate() {
                    let flag = if index == app.profiles.active {
                        MF_CHECKED
                    } else {
                        MF_UNCHECKED
                    };
                    let label = wide(&name.replace('&', "&&"));
                    AppendMenuW(
                        people,
                        MF_STRING | flag,
                        PROFILE_BASE + index,
                        PCWSTR(label.as_ptr()),
                    )?;
                }
                let pause = if app.profiles.paused {
                    w!("Retomar assinatura")
                } else {
                    w!("Pausar assinatura")
                };
                AppendMenuW(menu, MF_STRING, PAUSE, pause)?;
            }
            AppendMenuW(menu, MF_STRING, MANAGE, w!("Gerenciar nomes"))?;
            AppendMenuW(menu, MF_SEPARATOR, 0, None)?;
            AppendMenuW(menu, MF_STRING, EXIT, w!("Sair"))?;
            Ok(())
        });
        if result.is_ok() {
            let mut point = POINT::default();
            if GetCursorPos(&mut point).is_ok() {
                let _ = SetForegroundWindow(hwnd);
                let command = TrackPopupMenu(
                    menu,
                    TPM_RETURNCMD | TPM_RIGHTBUTTON,
                    point.x,
                    point.y,
                    None,
                    hwnd,
                    None,
                )
                .0;
                let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
                if command != 0 {
                    let _ =
                        PostMessageW(Some(hwnd), WM_COMMAND, WPARAM(command as usize), LPARAM(0));
                }
            }
        }
        let _ = DestroyMenu(menu);
        result
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message_id: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match message_id {
            SHOW => show(hwnd),
            WM_HOTKEY if wparam.0 == MANAGE_HOTKEY as usize => show(hwnd),
            TRAY => match lparam.0 as u32 {
                WM_LBUTTONDBLCLK => show(hwnd),
                WM_RBUTTONUP => {
                    let _ = tray_menu(hwnd);
                }
                _ => {}
            },
            WM_COMMAND
                if wparam.0 >> 16 == 0
                    || (wparam.0 & 0xffff == LIST && wparam.0 >> 16 == LBN_DBLCLK as usize)
                    || (wparam.0 & 0xffff == CLIENT
                        && wparam.0 >> 16 == CBN_SELCHANGE as usize) =>
            {
                let mut id = wparam.0 & 0xffff;
                if id == LIST {
                    id = SELECT;
                }
                match id {
                    MANAGE => show(hwnd),
                    EXIT => {
                        let _ = DestroyWindow(hwnd);
                    }
                    _ if id == 0 => {}
                    _ => {
                        let result =
                            APP.with_borrow_mut(|app| app.as_mut().map(|app| app.command(id)));
                        if let Some(Err(error)) = result {
                            message(&error);
                        }
                    }
                }
            }
            WM_TIMER => APP.with_borrow_mut(|app| {
                if let Some(app) = app
                    && !app.signing.poll()
                {
                    app.automation_available = false;
                    app.refresh(app.selected());
                }
            }),
            WM_CLOSE => {
                let tray_available =
                    APP.with_borrow(|app| app.as_ref().is_some_and(|app| app.notify(NIM_MODIFY)));
                if tray_available {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                } else {
                    let _ = DestroyWindow(hwnd);
                }
            }
            WM_QUERYENDSESSION => return LRESULT(1),
            WM_ENDSESSION if wparam.0 != 0 => {
                let _ = DestroyWindow(hwnd);
            }
            WM_DESTROY => {
                let _ = UnregisterHotKey(Some(hwnd), MANAGE_HOTKEY);
                let _ = KillTimer(Some(hwnd), 1);
                APP.with_borrow(|app| {
                    if let Some(app) = app {
                        app.notify(NIM_DELETE);
                    }
                });
                PostQuitMessage(0);
            }
            id if id == TASKBAR_CREATED.get() => {
                APP.with_borrow(|app| {
                    if let Some(app) = app {
                        app.notify(NIM_ADD);
                    }
                });
            }
            _ => return DefWindowProcW(hwnd, message_id, wparam, lparam),
        }
        LRESULT(0)
    }
}
