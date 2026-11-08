//! 未保存確認ダイアログ（OS ネイティブ呼び出し）。

/// OS 標準の保存確認ダイアログの選択結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmResult {
    /// 変更を保存して終了
    Save,
    /// 変更を保存せずに終了
    DontSave,
    /// キャンセル（終了を中止して作業を継続）
    Cancel,
}

/// Windows / OS 標準の保存確認ダイアログ（「保存して終了」「保存せず終了」「キャンセル」）を表示します。
#[cfg(windows)]
pub fn show_native_unsaved_confirm(doc_name: &str) -> ConfirmResult {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::atomic::{AtomicIsize, Ordering};

    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(hwnd: isize, lpText: *const u16, lpCaption: *const u16, uType: u32) -> i32;
        fn SetWindowsHookExW(
            idHook: i32,
            lpfn: unsafe extern "system" fn(i32, usize, isize) -> isize,
            hmod: isize,
            dwThreadId: u32,
        ) -> isize;
        fn UnhookWindowsHookEx(hhk: isize) -> i32;
        fn CallNextHookEx(hhk: isize, nCode: i32, wParam: usize, lParam: isize) -> isize;
        fn SetDlgItemTextW(hDlg: isize, nIDDlgItem: i32, lpString: *const u16) -> i32;
        fn GetDlgItem(hDlg: isize, nIDDlgItem: i32) -> isize;
        fn GetWindowRect(hWnd: isize, lpRect: *mut [i32; 4]) -> i32;
        fn SetWindowPos(hWnd: isize, hWndInsertAfter: isize, X: i32, Y: i32, cx: i32, cy: i32, uFlags: u32) -> i32;
        fn MapWindowPoints(hWndFrom: isize, hWndTo: isize, lpPoints: *mut [i32; 2], cPoints: u32) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentThreadId() -> u32;
    }

    static HOOK: AtomicIsize = AtomicIsize::new(0);

    unsafe extern "system" fn cbt_hook_proc(n_code: i32, w_param: usize, l_param: isize) -> isize {
        const HCBT_ACTIVATE: i32 = 5;
        const IDYES: i32 = 6;
        const IDNO: i32 = 7;
        const IDCANCEL: i32 = 2;
        const SWP_NOZORDER: u32 = 0x0004;
        const SWP_NOACTIVATE: u32 = 0x0010;

        if n_code == HCBT_ACTIVATE {
            let hwnd = w_param as isize;

            let to_w = |s: &str| -> Vec<u16> {
                OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
            };

            let s_yes = to_w("保存して終了(&S)");
            let s_no = to_w("保存せず終了(&N)");
            let s_cancel = to_w("キャンセル");

            // ボタンの文面を確実に「保存して終了」「保存せず終了」「キャンセル」に変更
            SetDlgItemTextW(hwnd, IDYES, s_yes.as_ptr());
            SetDlgItemTextW(hwnd, IDNO, s_no.as_ptr());
            SetDlgItemTextW(hwnd, IDCANCEL, s_cancel.as_ptr());

            // ボタン幅を 112px に拡張して日本語テキストが綺麗に収まるよう等間隔に配置
            let h_yes = GetDlgItem(hwnd, IDYES);
            let h_no = GetDlgItem(hwnd, IDNO);
            let h_cancel = GetDlgItem(hwnd, IDCANCEL);

            if h_yes != 0 && h_no != 0 && h_cancel != 0 {
                let mut r_dlg = [0i32; 4];
                let mut r_yes = [0i32; 4];
                let mut r_cancel = [0i32; 4];
                GetWindowRect(hwnd, &mut r_dlg);
                GetWindowRect(h_yes, &mut r_yes);
                GetWindowRect(h_cancel, &mut r_cancel);

                let dlg_width = r_dlg[2] - r_dlg[0];
                let btn_w = 112i32;
                let btn_h = (r_yes[3] - r_yes[1]).max(26);
                let gap = 10i32;
                let total_btns_w = btn_w * 3 + gap * 2;

                let required_dlg_w = (total_btns_w + 50).max(dlg_width);
                if required_dlg_w > dlg_width {
                    let dlg_h = r_dlg[3] - r_dlg[1];
                    SetWindowPos(hwnd, 0, r_dlg[0] - (required_dlg_w - dlg_width) / 2, r_dlg[1], required_dlg_w, dlg_h, SWP_NOZORDER | SWP_NOACTIVATE);
                }

                let mut pt_yes = [r_yes[0], r_yes[1]];
                MapWindowPoints(0, hwnd, &mut pt_yes, 1);
                let btn_y = pt_yes[1];

                let mut pt_cancel = [r_cancel[2], r_cancel[1]];
                MapWindowPoints(0, hwnd, &mut pt_cancel, 1);
                let right_edge = pt_cancel[0].max(required_dlg_w - 24);

                let x_cancel = right_edge - btn_w;
                let x_no = x_cancel - gap - btn_w;
                let x_yes = x_no - gap - btn_w;

                SetWindowPos(h_yes, 0, x_yes, btn_y, btn_w, btn_h, SWP_NOZORDER | SWP_NOACTIVATE);
                SetWindowPos(h_no, 0, x_no, btn_y, btn_w, btn_h, SWP_NOZORDER | SWP_NOACTIVATE);
                SetWindowPos(h_cancel, 0, x_cancel, btn_y, btn_w, btn_h, SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }

        let hhk = HOOK.load(Ordering::SeqCst);
        CallNextHookEx(hhk, n_code, w_param, l_param)
    }

    let to_wide = |s: &str| -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    };

    let title = to_wide("OpenAvi");
    let instruction = to_wide(&format!("「{doc_name}」への変更内容を保存しますか？\n\n保存せずに終了すると、編集内容はすべて失われます。"));

    const WH_CBT: i32 = 5;
    const MB_YESNOCANCEL: u32 = 0x00000003;
    const MB_ICONWARNING: u32 = 0x00000030;
    const MB_TOPMOST: u32 = 0x00040000;
    const IDYES: i32 = 6;
    const IDNO: i32 = 7;

    // 現在のスレッドに CBT フックを設置してボタンテキストを確実に変更
    let thread_id = unsafe { GetCurrentThreadId() };
    let hhk = unsafe { SetWindowsHookExW(WH_CBT, cbt_hook_proc, 0, thread_id) };
    HOOK.store(hhk, Ordering::SeqCst);

    let ret = unsafe { MessageBoxW(0, instruction.as_ptr(), title.as_ptr(), MB_YESNOCANCEL | MB_ICONWARNING | MB_TOPMOST) };

    if hhk != 0 {
        unsafe { UnhookWindowsHookEx(hhk) };
        HOOK.store(0, Ordering::SeqCst);
    }

    match ret {
        IDYES => ConfirmResult::Save,
        IDNO => ConfirmResult::DontSave,
        _ => ConfirmResult::Cancel,
    }
}

#[cfg(not(windows))]
pub fn show_native_unsaved_confirm(doc_name: &str) -> ConfirmResult {
    let res = rfd::MessageDialog::new()
        .set_title("OpenAvi")
        .set_description(&format!("「{doc_name}」への変更内容を保存しますか？"))
        .set_buttons(rfd::MessageButtons::YesNoCancel)
        .show();
    match res {
        rfd::MessageDialogResult::Yes => ConfirmResult::Save,
        rfd::MessageDialogResult::No => ConfirmResult::DontSave,
        _ => ConfirmResult::Cancel,
    }
}
