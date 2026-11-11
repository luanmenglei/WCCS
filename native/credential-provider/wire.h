#pragma once
#include <windows.h>

// Local-only message protocol. Packing and UTF-16 lengths match login_broker.rs.
// Query replies carry identifiers only. A consume reply carries a single-use secret.
#pragma pack(push, 1)
struct LoginRequest
{
    DWORD version;
    DWORD verb;
    char device[65];
    char instance[65];
    char operation[65];
};
struct LoginResponse
{
    DWORD version;
    DWORD status;
    ULONGLONG deadline;
    char device[65];
    char instance[65];
    char operation[65];
    BYTE reserved;
    wchar_t username[64];
    wchar_t sid[192];
    wchar_t password[128];
};
#pragma pack(pop)
static_assert(sizeof(LoginRequest) == 203);
static_assert(sizeof(LoginResponse) == 980);
inline constexpr wchar_t PipeName[] = L"\\\\.\\pipe\\WCCS-Contest-Login-v1";
