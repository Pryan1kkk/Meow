//! Иконка в системном трее и скрытие/показ окна. Только Windows.
//!
//! Окно прячется и показывается напрямую через Win32 (`ShowWindow`), а не через
//! команды eframe: у скрытого окна цикл перерисовки не работает, и команда
//! «показать» просто не была бы обработана.
#![allow(dead_code)]

/// Держит иконку в трее живой, пока существует это значение
pub struct Tray {
    #[cfg(windows)]
    _icon: tray_icon::TrayIcon,
}

#[allow(unused_imports)]
pub use imp::{create, hide_window, show_window};

#[cfg(windows)]
mod imp {
    use super::Tray;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SetForegroundWindow, ShowWindow, SW_HIDE, SW_RESTORE,
    };

    /// Должно совпадать с заголовком окна в `main()`
    const WINDOW_TITLE: &str = "TaskMgr";

    fn find_hwnd() -> HWND {
        let title: Vec<u16> = WINDOW_TITLE.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) }
    }

    pub fn hide_window() {
        let h = find_hwnd();
        if !h.is_null() {
            unsafe {
                ShowWindow(h, SW_HIDE);
            }
        }
    }

    pub fn show_window() {
        let h = find_hwnd();
        if !h.is_null() {
            unsafe {
                ShowWindow(h, SW_RESTORE);
                SetForegroundWindow(h);
            }
        }
    }

    /// Рисуем иконку кодом: синий скруглённый квадрат с тремя белыми столбиками
    fn make_icon() -> Option<Icon> {
        const N: usize = 32;
        const R: f32 = 7.0;
        let bars: [(usize, usize, usize, usize); 3] = [(7, 11, 15, 25), (14, 18, 8, 25), (21, 25, 12, 25)];
        let mut px = vec![0u8; N * N * 4];
        for y in 0..N {
            for x in 0..N {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let cx = fx.clamp(R, N as f32 - R);
                let cy = fy.clamp(R, N as f32 - R);
                if ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt() > R {
                    continue;
                }
                let in_bar = bars.iter().any(|&(x0, x1, y0, y1)| x >= x0 && x < x1 && y >= y0 && y < y1);
                let c: [u8; 4] = if in_bar { [255, 255, 255, 255] } else { [79, 140, 255, 255] };
                let i = (y * N + x) * 4;
                px[i..i + 4].copy_from_slice(&c);
            }
        }
        Icon::from_rgba(px, N as u32, N as u32).ok()
    }

    /// Создаёт иконку в трее. Вызывать из главного потока (из `eframe` creator).
    pub fn create() -> Option<Tray> {
        let show = MenuItem::new("Показать", true, None);
        let quit = MenuItem::new("Выход", true, None);
        let menu = Menu::new();
        menu.append(&show).ok()?;
        menu.append(&quit).ok()?;

        let show_id = show.id().clone();
        let quit_id = quit.id().clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            if e.id == show_id {
                show_window();
            } else if e.id == quit_id {
                std::process::exit(0);
            }
        }));
        TrayIconEvent::set_event_handler(Some(|e: TrayIconEvent| match e {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
            | TrayIconEvent::DoubleClick { .. } => show_window(),
            _ => {}
        }));

        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip("TaskMgr")
            .with_icon(make_icon()?)
            .build()
            .ok()?;
        Some(Tray { _icon: icon })
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Tray;

    pub fn create() -> Option<Tray> {
        None
    }
    pub fn hide_window() {}
    pub fn show_window() {}
}
