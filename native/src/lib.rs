//! dwmapi.dll 代理 + Win32 身份 API 钩子。
//!
//! 设计要点：完全不引用 PCL 内部符号（无类型名、无 RVA、无 IL 偏移），
//! 只依赖 Windows 用户身份 API，因此跨 PCL 版本自愈。
//!
//! 判据链条（见 PCL.Core/App/Region/RegionPolicy.cs）：
//!   geo-cn           <- GetUserDefaultGeoName
//!   device-region-cn <- RegOpenKeyExW(...\Control Panel\DeviceRegion)
//!   timezone+locale  <- GetUserDefaultLocaleName / GetUserPreferredUILanguages
//! 我们把前三个信号改写成「美国 + en-US + 设备区域不可读」，
//! Evaluate 就会走 geo-other 分支返回 Other，IsRestricted 变 false。
//!
//! 为什么不在 DllMain 里起线程：DllMain 执行期间加载器锁被持有，
//! 新线程一旦调用 LoadLibraryExW / GetProcAddress 就会死锁。
//! 因此改为「第一次被导出函数调用时」安装钩子——此时调用来自应用代码，锁已释放。

#![allow(non_snake_case, clippy::missing_safety_doc)]

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::Once;

type Handle = *mut c_void;

const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x0000_0800;
const PAGE_EXECUTE_READWRITE: u32 = 0x40;
const ERROR_FILE_NOT_FOUND: i32 = 2;

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(name: *const u16, file: Handle, flags: u32) -> Handle;
    fn GetProcAddress(module: Handle, name: *const u8) -> *mut c_void;
    fn GetModuleHandleW(name: *const u16) -> Handle;
    fn VirtualProtect(addr: *mut c_void, size: usize, new: u32, old: *mut u32) -> i32;
    fn FlushInstructionCache(process: Handle, addr: *mut c_void, size: usize) -> i32;
    fn GetCurrentProcess() -> Handle;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetSystemDirectoryW(buf: *mut u16, size: u32) -> u32;
    fn GetModuleFileNameW(module: Handle, buf: *mut u16, size: u32) -> u32;
    fn GetErrorMode() -> u32;
    fn SetErrorMode(mode: u32) -> u32;
}

/// 让加载器不再弹出「无法定位序数 / 找不到入口点」这类模态框。
/// 置位之后缺失导入只会静默失败，进程继续运行，行为与应用原本的容错一致。
const SEM_FAILCRITICALERRORS: u32 = 0x0001;

static DONE: AtomicBool = AtomicBool::new(false);
static REAL_REG_OPEN: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static PATCH: Once = Once::new();

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn write_wide(dst: *mut u16, count: i32, text: &str) -> i32 {
    let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    if count <= 0 || dst.is_null() || units.len() > count as usize {
        return 0;
    }
    unsafe { std::ptr::copy_nonoverlapping(units.as_ptr(), dst, units.len()) };
    units.len() as i32
}

fn log(msg: &str) {
    use std::io::Write;
    let Ok(tmp) = std::env::var("TEMP") else { return };
    let path = std::path::Path::new(&tmp).join("pcl-region-unlock.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "[dwmapi-proxy] {msg}");
    }
}

// ---------------------------------------------------------------- 转发真实 dwmapi

fn real_dwm() -> Handle {
    static CACHE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
    unsafe {
        let cached = CACHE.load(Ordering::SeqCst);
        if !cached.is_null() {
            return cached;
        }

        // 必须用 System32 的完整路径。只给 "dwmapi.dll" 时加载器会按基础名
        // 匹配到我们自己的模块，转发就变成自递归。
        let mut sysdir = [0u16; 260];
        let n = GetSystemDirectoryW(sysdir.as_mut_ptr(), sysdir.len() as u32) as usize;
        let mut path: Vec<u16> = sysdir[..n].to_vec();
        path.extend("\\dwmapi.dll".encode_utf16());
        path.push(0);

        let module = LoadLibraryExW(path.as_ptr(), std::ptr::null_mut(), 0);
        CACHE.store(module, Ordering::SeqCst);

        let mut buf = [0u16; 512];
        let len = GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) as usize;
        log(&format!(
            "real dwmapi = {} (handle {module:p})",
            String::from_utf16_lossy(&buf[..len])
        ));

        module
    }
}

unsafe fn real_fn(name: &'static str) -> *mut c_void {
    let key = format!("{name}\0");
    GetProcAddress(real_dwm(), key.as_ptr())
}

macro_rules! forward {
    (
        $( fn $name:ident($($arg:ident : $ty:ty),*) -> $ret:ty ; )*
    ) => {
        $(
            #[no_mangle]
            pub unsafe extern "system" fn $name($($arg: $ty),*) -> $ret {
                // 记录并恢复 last-error，安装钩子的副作用不应该泄漏给调用方。
                let saved = GetLastError();
                ensure_patched();
                SetLastError(saved);

                type Fn = unsafe extern "system" fn($($ty),*) -> $ret;
                let p = real_fn(stringify!($name));
                if p.is_null() {
                    return Default::default();
                }
                let f: Fn = std::mem::transmute(p);
                f($($arg),*)
            }
        )*
    };
}

forward! {
    fn DwmIsCompositionEnabled(enabled: *mut i32) -> i32;
    fn DwmExtendFrameIntoClientArea(hwnd: Handle, margins: *const c_void) -> i32;
    fn DwmSetWindowAttribute(hwnd: Handle, attr: u32, value: *const c_void, size: u32) -> i32;
    fn DwmGetWindowAttribute(hwnd: Handle, attr: u32, value: *mut c_void, size: u32) -> i32;
    fn DwmEnableBlurBehindWindow(hwnd: Handle, bb: *const c_void) -> i32;
    fn DwmFlush() -> i32;
    fn DwmDefWindowProc(hwnd: Handle, msg: u32, wparam: usize, lparam: isize, result: *mut isize) -> i32;
    fn DwmGetColorizationColor(color: *mut u32, opaque: *mut i32) -> i32;
}

// ---------------------------------------------------------------- 身份 API 钩子

unsafe extern "system" fn hook_geo_name(buf: *mut u16, count: i32) -> i32 {
    write_wide(buf, count, "US")
}

unsafe extern "system" fn hook_locale_name(buf: *mut u16, count: i32) -> i32 {
    write_wide(buf, count, "en-US")
}

unsafe extern "system" fn hook_ui_languages(
    _flags: u32, count: *mut u32, buf: *mut u16, chars: *mut u32,
) -> i32 {
    const NEEDED: u32 = 6; // "en-US" + NUL
    if chars.is_null() {
        return 0;
    }
    if buf.is_null() {
        unsafe {
            *chars = NEEDED;
            if !count.is_null() {
                *count = 1;
            }
        }
        return 1;
    }
    if unsafe { *chars } < NEEDED {
        return 0;
    }
    if write_wide(buf, NEEDED as i32, "en-US") == 0 {
        return 0;
    }
    unsafe {
        *chars = NEEDED;
        if !count.is_null() {
            *count = 1;
        }
    }
    1
}

unsafe extern "system" fn hook_reg_open_key(
    hkey: usize, subkey: *const u16, options: u32, sam: u32, result: *mut usize,
) -> i32 {
    // 让 DeviceRegion 键「打不开」，TryReadDeviceRegion 会视为信号不可用。
    if !subkey.is_null() {
        let mut len = 0usize;
        while unsafe { *subkey.add(len) } != 0 && len < 512 {
            len += 1;
        }
        let slice = unsafe { std::slice::from_raw_parts(subkey, len) };
        if String::from_utf16_lossy(slice).contains("DeviceRegion") {
            return ERROR_FILE_NOT_FOUND;
        }
    }

    type Fn = unsafe extern "system" fn(usize, *const u16, u32, u32, *mut usize) -> i32;
    let real = REAL_REG_OPEN.load(Ordering::SeqCst);
    if real.is_null() {
        return ERROR_FILE_NOT_FOUND;
    }
    let f: Fn = unsafe { std::mem::transmute(real) };
    f(hkey, subkey, options, sam, result)
}

/// 在目标函数入口写入 `mov rax, imm64; jmp rax`。
unsafe fn install(target: *mut c_void, replacement: *mut c_void) {
    if target.is_null() || replacement.is_null() {
        return;
    }
    let mut old = 0u32;
    if unsafe { VirtualProtect(target, 12, PAGE_EXECUTE_READWRITE, &mut old) } == 0 {
        return;
    }
    let code = target as *mut u8;
    unsafe {
        *code = 0x48;
        *code.add(1) = 0xB8;
        std::ptr::write_unaligned(code.add(2) as *mut i64, replacement as i64);
        *code.add(10) = 0xFF;
        *code.add(11) = 0xE0;
        let mut ignored = 0u32;
        VirtualProtect(target, 12, old, &mut ignored);
        FlushInstructionCache(GetCurrentProcess(), target, 12);
    }
}

fn patch_module(module_name: &str, exports: &[(&str, *mut c_void)]) {
    let mut handle = unsafe { GetModuleHandleW(wide(module_name).as_ptr()) };
    if handle.is_null() {
        handle = unsafe {
            LoadLibraryExW(wide(module_name).as_ptr(), std::ptr::null_mut(),
                           LOAD_LIBRARY_SEARCH_SYSTEM32)
        };
    }
    if handle.is_null() {
        log(&format!("{module_name}: module not found"));
        return;
    }

    for (name, replacement) in exports {
        let key = format!("{name}\0");
        let target = unsafe { GetProcAddress(handle, key.as_ptr()) };
        if target.is_null() {
            log(&format!("{module_name}!{name}: export not found"));
            continue;
        }
        unsafe { install(target, *replacement) };
        log(&format!("hooked {module_name}!{name} @ {target:p}"));
    }
}

/// 首次被调用时安装钩子。此时不处于 DllMain，可以安全调用加载器 API。
fn ensure_patched() {
    PATCH.call_once(|| {
        if DONE.swap(true, Ordering::SeqCst) {
            return;
        }

        // 必须在装钩子之前拿到真实的 RegOpenKeyExW，
        // 否则 GetProcAddress 会返回我们自己写进去的跳转，导致无限递归。
        let mut real_reg = std::ptr::null_mut();
        unsafe {
            let kb = GetModuleHandleW(wide("kernelbase.dll").as_ptr());
            if !kb.is_null() {
                real_reg = GetProcAddress(kb, b"RegOpenKeyExW\0".as_ptr());
            }
        }
        REAL_REG_OPEN.store(real_reg, Ordering::SeqCst);
        log(&format!("real RegOpenKeyExW @ {real_reg:p}"));

        // RegOpenKeyExW 的钩子需要转发给真实实现，因此绝不能钩 kernelbase：
        // kernelbase 的导出就是真实现本尊，钩掉它之后 REAL_REG_OPEN 会指向我们自己，
        // 转发表就变成自递归。kernel32 / advapi32 的导出是跳板，钩它们是安全的。
        let identity: [(&str, *mut c_void); 3] = [
            ("GetUserDefaultGeoName", hook_geo_name as *mut c_void),
            ("GetUserDefaultLocaleName", hook_locale_name as *mut c_void),
            ("GetUserPreferredUILanguages", hook_ui_languages as *mut c_void),
        ];
        let registry: [(&str, *mut c_void); 1] = [
            ("RegOpenKeyExW", hook_reg_open_key as *mut c_void),
        ];

        patch_module("kernel32.dll", &identity);
        patch_module("kernelbase.dll", &identity);
        patch_module("kernel32.dll", &registry);
        patch_module("advapi32.dll", &registry);
        log("install finished");
    });
}

#[no_mangle]
pub extern "system" fn DllMain(_inst: Handle, reason: u32, _reserved: *mut c_void) -> i32 {
    // DLL_PROCESS_ATTACH：尽早静音加载器模态框。
    // SetErrorMode 不涉及模块加载，在 DllMain 里调用是安全的。
    const DLL_PROCESS_ATTACH: u32 = 1;
    if reason == DLL_PROCESS_ATTACH {
        unsafe {
            let previous = GetErrorMode();
            SetErrorMode(previous | SEM_FAILCRITICALERRORS);
        }
    }
    // 其余工作有意留空：DllMain 期间加载器锁被持有，
    // 任何 LoadLibrary / GetProcAddress 都可能死锁。
    1
}
