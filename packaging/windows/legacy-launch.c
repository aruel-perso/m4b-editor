// Compatibility entry point for updater helpers that relaunch the old path.
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <wchar.h>

int WINAPI wWinMain(HINSTANCE instance, HINSTANCE previous, PWSTR arguments, int show)
{
    (void)instance;
    (void)previous;
    (void)show;
    wchar_t executable[32768];
    DWORD bytes = sizeof executable;
    if (RegGetValueW(HKEY_CURRENT_USER, L"Software\\M4B-Editor", L"Executable",
                     RRF_RT_REG_SZ, NULL, executable, &bytes) != ERROR_SUCCESS)
        return 1;

    wchar_t command[32768];
    if (swprintf(command, ARRAYSIZE(command), L"\"%ls\" %ls", executable, arguments) < 0)
        return 1;

    STARTUPINFOW startup = {0};
    startup.cb = sizeof startup;
    PROCESS_INFORMATION child = {0};
    if (!CreateProcessW(executable, command, NULL, NULL, FALSE, 0, NULL, NULL, &startup, &child))
        return 1;
    CloseHandle(child.hThread);
    CloseHandle(child.hProcess);
    return 0;
}
