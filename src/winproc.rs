//! Действия над процессами, которых нет в `sysinfo`: приостановка/возобновление и приоритет.
//! Работает только в Windows; на других ОС функции возвращают ошибку.
#![allow(dead_code)]

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Priority {
    Idle,
    BelowNormal,
    Normal,
    AboveNormal,
    High,
}

impl Priority {
    /// Realtime намеренно не предлагаем: он может подвесить систему целиком.
    pub const ALL: [Priority; 5] = [
        Priority::Idle,
        Priority::BelowNormal,
        Priority::Normal,
        Priority::AboveNormal,
        Priority::High,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Priority::Idle => "Низкий",
            Priority::BelowNormal => "Ниже среднего",
            Priority::Normal => "Средний",
            Priority::AboveNormal => "Выше среднего",
            Priority::High => "Высокий",
        }
    }
}

pub use imp::{get_priority, resume, set_priority, suspend};

#[cfg(windows)]
mod imp {
    use super::Priority;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::Threading::{
        ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS, GetPriorityClass, HIGH_PRIORITY_CLASS,
        IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_SET_INFORMATION, PROCESS_SUSPEND_RESUME, SetPriorityClass,
    };

    // NtSuspendProcess / NtResumeProcess лежат в ntdll и в windows-sys не описаны
    windows_link::link!("ntdll.dll" "system" fn NtSuspendProcess(process : HANDLE) -> i32);
    windows_link::link!("ntdll.dll" "system" fn NtResumeProcess(process : HANDLE) -> i32);

    /// Закрывает дескриптор при выходе из области видимости
    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    fn open(pid: u32, access: u32) -> Result<Handle, String> {
        let h = unsafe { OpenProcess(access, 0, pid) };
        if h.is_null() {
            let err = std::io::Error::last_os_error();
            Err(if err.raw_os_error() == Some(5) {
                "отказано в доступе (запусти от имени администратора)".to_owned()
            } else {
                err.to_string()
            })
        } else {
            Ok(Handle(h))
        }
    }

    fn nt_result(status: i32) -> Result<(), String> {
        // NTSTATUS: отрицательные значения — ошибки
        if status < 0 {
            Err(format!("ошибка NTSTATUS 0x{:08X}", status as u32))
        } else {
            Ok(())
        }
    }

    pub fn suspend(pid: u32) -> Result<(), String> {
        let h = open(pid, PROCESS_SUSPEND_RESUME)?;
        nt_result(unsafe { NtSuspendProcess(h.0) })
    }

    pub fn resume(pid: u32) -> Result<(), String> {
        let h = open(pid, PROCESS_SUSPEND_RESUME)?;
        nt_result(unsafe { NtResumeProcess(h.0) })
    }

    pub fn get_priority(pid: u32) -> Option<Priority> {
        let h = open(pid, PROCESS_QUERY_LIMITED_INFORMATION).ok()?;
        let class = unsafe { GetPriorityClass(h.0) };
        match class {
            IDLE_PRIORITY_CLASS => Some(Priority::Idle),
            BELOW_NORMAL_PRIORITY_CLASS => Some(Priority::BelowNormal),
            NORMAL_PRIORITY_CLASS => Some(Priority::Normal),
            ABOVE_NORMAL_PRIORITY_CLASS => Some(Priority::AboveNormal),
            HIGH_PRIORITY_CLASS => Some(Priority::High),
            _ => None, // Realtime или неизвестный класс
        }
    }

    pub fn set_priority(pid: u32, p: Priority) -> Result<(), String> {
        let class = match p {
            Priority::Idle => IDLE_PRIORITY_CLASS,
            Priority::BelowNormal => BELOW_NORMAL_PRIORITY_CLASS,
            Priority::Normal => NORMAL_PRIORITY_CLASS,
            Priority::AboveNormal => ABOVE_NORMAL_PRIORITY_CLASS,
            Priority::High => HIGH_PRIORITY_CLASS,
        };
        let h = open(pid, PROCESS_SET_INFORMATION)?;
        if unsafe { SetPriorityClass(h.0, class) } == 0 {
            Err(std::io::Error::last_os_error().to_string())
        } else {
            Ok(())
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Priority;

    const NOT_WIN: &str = "доступно только в Windows";

    pub fn suspend(_pid: u32) -> Result<(), String> {
        Err(NOT_WIN.to_owned())
    }
    pub fn resume(_pid: u32) -> Result<(), String> {
        Err(NOT_WIN.to_owned())
    }
    pub fn get_priority(_pid: u32) -> Option<Priority> {
        None
    }
    pub fn set_priority(_pid: u32, _p: Priority) -> Result<(), String> {
        Err(NOT_WIN.to_owned())
    }
}
