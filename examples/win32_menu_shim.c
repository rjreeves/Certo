/* Thin ABI shim between Certo's `extern "C"` FFI (which only has Int/Text/
 * Bool primitives — no HANDLE/UINT/LPCSTR types) and the real Win32 popup-
 * menu API. Every wrapper here takes/returns plain 64-bit ints (and a plain
 * char* for text) so the prototype Certo's codegen emits for each function
 * matches this file's own definition exactly, and none of these names
 * collide with anything already declared by <windows.h> (which Certo's own
 * generated C always includes) the way calling the real WinAPI names
 * directly does — see BACKLOG.md for that gap.
 */
#include <windows.h>
#include <stdint.h>

/* A dedicated, real top-level window to own the popup menu — NOT the
 * console window. Under a modern terminal host (Windows Terminal, VS
 * Code's integrated terminal), GetConsoleWindow() returns a handle to a
 * hidden conhost.exe window that never actually becomes the real
 * foreground window (the visible terminal is a separate process/HWND
 * entirely); TrackPopupMenu anchored to it loses activation the instant
 * it opens and self-dismisses before a click can land. Creating our own
 * invisible top-level window and foregrounding *that* instead is the
 * same well-known pattern every system-tray-icon context menu uses. */
static HWND certo_ctx_menu_host_window(void) {
    static HWND hwnd = NULL;
    if (hwnd) return hwnd;
    WNDCLASSA wc = {0};
    wc.lpfnWndProc   = DefWindowProcA;
    wc.hInstance     = GetModuleHandleA(NULL);
    wc.lpszClassName = "CertoContextMenuHost";
    RegisterClassA(&wc);
    hwnd = CreateWindowExA(WS_EX_TOOLWINDOW, "CertoContextMenuHost", "",
                            WS_POPUP, 0, 0, 0, 0, NULL, NULL, wc.hInstance, NULL);
    return hwnd;
}

long long certo_win32_get_host_window(void) {
    return (long long)(intptr_t)certo_ctx_menu_host_window();
}

long long certo_win32_set_foreground_window(long long hwnd) {
    return (long long)SetForegroundWindow((HWND)(intptr_t)hwnd);
}

long long certo_win32_create_popup_menu(void) {
    return (long long)(intptr_t)CreatePopupMenu();
}

long long certo_win32_append_menu(long long hMenu, long long uFlags, long long uId, const char* text) {
    return (long long)AppendMenuA((HMENU)(intptr_t)hMenu, (UINT)uFlags, (UINT_PTR)uId, text);
}

long long certo_win32_track_popup_menu(long long hMenu, long long uFlags, long long x, long long y, long long hwnd) {
    long long result = (long long)TrackPopupMenu((HMENU)(intptr_t)hMenu, (UINT)uFlags, (int)x, (int)y, 0, (HWND)(intptr_t)hwnd, NULL);
    /* Documented Win32 workaround (Microsoft KB135788): without this, the
     * menu can fail to close cleanly / a spurious message can reach the
     * host window immediately afterward. */
    PostMessageA((HWND)(intptr_t)hwnd, WM_NULL, 0, 0);
    return result;
}

long long certo_win32_destroy_menu(long long hMenu) {
    return (long long)DestroyMenu((HMENU)(intptr_t)hMenu);
}
