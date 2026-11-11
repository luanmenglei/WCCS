// WCCS Credential Provider V2. Windows SDK COM/LogonUI boundary.
// Callbacks and the hidden timer window stay on LogonUI's advising thread.
#define SECURITY_WIN32
#include "wire.h"
#include <credentialprovider.h>
#include <new>
#include <ntsecapi.h>
#include <security.h>
#include <strsafe.h>
#include <wincred.h>
#include <windows.h>

static const GUID ProviderId = {0x954886e2, 0x4dc2, 0x4e85, {0xa9, 0x62, 0x26, 0x87, 0xf7, 0x65, 0x8e, 0x0c}};
static HMODULE module;
static LONG objects;

struct Handle
{
    HANDLE value;
    explicit Handle(HANDLE handle = nullptr) : value(handle)
    {
    }
    ~Handle()
    {
        if (value && value != INVALID_HANDLE_VALUE)
            CloseHandle(value);
    }
    Handle(const Handle &) = delete;
    Handle &operator=(const Handle &) = delete;
};
struct SecretResponse
{
    alignas(wchar_t) LoginResponse value{};
    ~SecretResponse()
    {
        SecureZeroMemory(&value, sizeof(value));
    }
};
static HRESULT CopyText(PCWSTR source, PWSTR *destination)
{
    const size_t size = (wcslen(source) + 1) * sizeof(wchar_t);
    *destination = static_cast<PWSTR>(CoTaskMemAlloc(size));
    if (!*destination)
        return E_OUTOFMEMORY;
    CopyMemory(*destination, source, size);
    return S_OK;
}
static bool IsService(HANDLE pipe)
{
    ULONG pid = 0;
    if (!GetNamedPipeServerProcessId(pipe, &pid))
        return false;
    Handle process(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid));
    if (!process.value)
        return false;
    wchar_t expected[32768]{};
    if (!GetModuleFileNameW(module, expected, ARRAYSIZE(expected)))
        return false;
    wchar_t *slash = wcsrchr(expected, L'\\');
    if (!slash)
        return false;
    *slash = 0;
    if (FAILED(StringCchCatW(expected, ARRAYSIZE(expected), L"\\wccs-agent.exe")))
        return false;
    wchar_t path[32768]{};
    DWORD length = ARRAYSIZE(path);
    if (!QueryFullProcessImageNameW(process.value, 0, path, &length) || _wcsicmp(path, expected))
        return false;
    HANDLE rawToken = nullptr;
    if (!OpenProcessToken(process.value, TOKEN_QUERY, &rawToken))
        return false;
    Handle token(rawToken);
    alignas(TOKEN_USER) BYTE buffer[1024]{};
    DWORD bytes = 0;
    if (!GetTokenInformation(token.value, TokenUser, buffer, sizeof(buffer), &bytes))
        return false;
    return IsWellKnownSid(reinterpret_cast<TOKEN_USER *>(buffer)->User.Sid, WinLocalSystemSid) != FALSE;
}
static bool Transfer(HANDLE pipe, void *buffer, DWORD size, bool writing)
{
    Handle event(CreateEventW(nullptr, TRUE, FALSE, nullptr));
    if (!event.value)
        return false;
    OVERLAPPED overlap{};
    overlap.hEvent = event.value;
    DWORD bytes = 0;
    BOOL result =
        writing ? WriteFile(pipe, buffer, size, &bytes, &overlap) : ReadFile(pipe, buffer, size, &bytes, &overlap);
    if (!result && GetLastError() != ERROR_IO_PENDING)
        return false;
    if (!result && WaitForSingleObject(event.value, 200) != WAIT_OBJECT_0)
    {
        CancelIoEx(pipe, &overlap);
        // The OVERLAPPED storage and event outlive cancellation completion.
        GetOverlappedResult(pipe, &overlap, &bytes, TRUE);
        return false;
    }
    return GetOverlappedResult(pipe, &overlap, &bytes, FALSE) && bytes == size;
}
static bool Exchange(const LoginRequest &request, LoginResponse &response)
{
    auto open = []() {
        return CreateFileW(PipeName, GENERIC_READ | GENERIC_WRITE, 0, nullptr, OPEN_EXISTING,
                           FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION, nullptr);
    };
    HANDLE rawPipe = open();
    // CredentialsChanged can synchronously request serialization immediately after a query.
    // Let the broker observe the closed query handle before opening the one-shot consume channel.
    if (rawPipe == INVALID_HANDLE_VALUE && GetLastError() == ERROR_PIPE_BUSY && WaitNamedPipeW(PipeName, 200))
        rawPipe = open();
    Handle pipe(rawPipe);
    if (pipe.value == INVALID_HANDLE_VALUE || !IsService(pipe.value))
        return false;
    DWORD mode = PIPE_READMODE_MESSAGE;
    if (!SetNamedPipeHandleState(pipe.value, &mode, nullptr, nullptr))
        return false;
    LoginRequest copy = request;
    if (!Transfer(pipe.value, &copy, sizeof(copy), true) || !Transfer(pipe.value, &response, sizeof(response), false))
        return false;
    return response.version == 1 && response.status == 1 && response.deadline > GetTickCount64() &&
           response.device[64] == 0 && response.instance[64] == 0 && response.operation[64] == 0 &&
           response.username[63] == 0 && response.sid[191] == 0 && response.password[127] == 0;
}

static HRESULT Serialize(LoginResponse &secret, CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION *output)
{
    wchar_t protectedPassword[512]{};
    DWORD capacity = ARRAYSIZE(protectedPassword);
    if (!CredProtectW(FALSE, secret.password, static_cast<DWORD>(wcslen(secret.password) + 1), protectedPassword,
                      &capacity, nullptr))
    {
        HRESULT error = HRESULT_FROM_WIN32(GetLastError());
        SecureZeroMemory(protectedPassword, sizeof(protectedPassword));
        return error;
    }
    SecureZeroMemory(secret.password, sizeof(secret.password));
    wchar_t computer[256]{};
    DWORD computerSize = ARRAYSIZE(computer);
    if (!GetComputerNameW(computer, &computerSize))
    {
        SecureZeroMemory(protectedPassword, sizeof(protectedPassword));
        return E_FAIL;
    }
    const DWORD bytes =
        static_cast<DWORD>(sizeof(KERB_INTERACTIVE_UNLOCK_LOGON) +
                           (wcslen(computer) + wcslen(secret.username) + wcslen(protectedPassword)) * sizeof(wchar_t));
    auto packed = static_cast<KERB_INTERACTIVE_UNLOCK_LOGON *>(CoTaskMemAlloc(bytes));
    if (!packed)
    {
        SecureZeroMemory(protectedPassword, sizeof(protectedPassword));
        return E_OUTOFMEMORY;
    }
    ZeroMemory(packed, bytes);
    packed->Logon.MessageType = KerbInteractiveLogon;
    BYTE *cursor = reinterpret_cast<BYTE *>(packed) + sizeof(*packed);
    auto append = [&](UNICODE_STRING &target, PCWSTR source) {
        target.Length = static_cast<USHORT>(wcslen(source) * sizeof(wchar_t));
        target.MaximumLength = target.Length;
        CopyMemory(cursor, source, target.Length);
        target.Buffer = reinterpret_cast<PWSTR>(cursor - reinterpret_cast<BYTE *>(packed));
        cursor += target.Length;
    };
    append(packed->Logon.LogonDomainName, computer);
    append(packed->Logon.UserName, secret.username);
    append(packed->Logon.Password, protectedPassword);
    SecureZeroMemory(protectedPassword, sizeof(protectedPassword));
    LSA_HANDLE lsa = nullptr;
    NTSTATUS status = LsaConnectUntrusted(&lsa);
    ULONG package = 0;
    if (status >= 0)
    {
        char name[] = "Negotiate";
        LSA_STRING nameString{static_cast<USHORT>(sizeof(name) - 1), static_cast<USHORT>(sizeof(name)), name};
        status = LsaLookupAuthenticationPackage(lsa, &nameString, &package);
        LsaDeregisterLogonProcess(lsa);
    }
    if (status < 0)
    {
        SecureZeroMemory(packed, bytes);
        CoTaskMemFree(packed);
        return HRESULT_FROM_WIN32(LsaNtStatusToWinError(status));
    }
    output->ulAuthenticationPackage = package;
    output->clsidCredentialProvider = ProviderId;
    output->cbSerialization = bytes;
    // LogonUI takes ownership of this CoTaskMem allocation on successful serialization.
    output->rgbSerialization = reinterpret_cast<BYTE *>(packed);
    return S_OK;
}

class Credential final : public ICredentialProviderCredential2
{
    LONG references = 1;
    alignas(wchar_t) LoginResponse metadata{};

  public:
    explicit Credential(const LoginResponse &value) : metadata(value)
    {
        InterlockedIncrement(&objects);
    }
    ~Credential()
    {
        SecureZeroMemory(&metadata, sizeof(metadata));
        InterlockedDecrement(&objects);
    }
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID id, void **result) override
    {
        if (!result)
            return E_POINTER;
        *result = nullptr;
        if (id != IID_IUnknown && id != IID_ICredentialProviderCredential && id != IID_ICredentialProviderCredential2)
            return E_NOINTERFACE;
        *result = static_cast<ICredentialProviderCredential2 *>(this);
        AddRef();
        return S_OK;
    }
    ULONG STDMETHODCALLTYPE AddRef() override
    {
        return InterlockedIncrement(&references);
    }
    ULONG STDMETHODCALLTYPE Release() override
    {
        ULONG count = InterlockedDecrement(&references);
        if (!count)
            delete this;
        return count;
    }
    HRESULT STDMETHODCALLTYPE Advise(ICredentialProviderCredentialEvents *) override
    {
        return S_OK;
    }
    HRESULT STDMETHODCALLTYPE UnAdvise() override
    {
        return S_OK;
    }
    HRESULT STDMETHODCALLTYPE SetSelected(BOOL *automatic) override
    {
        *automatic = FALSE;
        return S_OK;
    }
    HRESULT STDMETHODCALLTYPE SetDeselected() override
    {
        return S_OK;
    }
    HRESULT STDMETHODCALLTYPE GetFieldState(DWORD id, CREDENTIAL_PROVIDER_FIELD_STATE *state,
                                            CREDENTIAL_PROVIDER_FIELD_INTERACTIVE_STATE *interaction) override
    {
        if (id != 0)
            return E_INVALIDARG;
        *state = CPFS_DISPLAY_IN_BOTH;
        *interaction = CPFIS_NONE;
        return S_OK;
    }
    HRESULT STDMETHODCALLTYPE GetStringValue(DWORD id, PWSTR *value) override
    {
        return id == 0 ? CopyText(L"WCCS contest sign-in", value) : E_INVALIDARG;
    }
    HRESULT STDMETHODCALLTYPE GetBitmapValue(DWORD, HBITMAP *) override
    {
        return E_NOTIMPL;
    }
    HRESULT STDMETHODCALLTYPE GetCheckboxValue(DWORD, BOOL *, PWSTR *) override
    {
        return E_NOTIMPL;
    }
    HRESULT STDMETHODCALLTYPE GetSubmitButtonValue(DWORD, DWORD *) override
    {
        return E_NOTIMPL;
    }
    HRESULT STDMETHODCALLTYPE GetComboBoxValueCount(DWORD, DWORD *, DWORD *) override
    {
        return E_NOTIMPL;
    }
    HRESULT STDMETHODCALLTYPE GetComboBoxValueAt(DWORD, DWORD, PWSTR *) override
    {
        return E_NOTIMPL;
    }
    HRESULT STDMETHODCALLTYPE SetStringValue(DWORD, PCWSTR) override
    {
        return E_NOTIMPL;
    }
    HRESULT STDMETHODCALLTYPE SetCheckboxValue(DWORD, BOOL) override
    {
        return E_NOTIMPL;
    }
    HRESULT STDMETHODCALLTYPE SetComboBoxSelectedValue(DWORD, DWORD) override
    {
        return E_NOTIMPL;
    }
    HRESULT STDMETHODCALLTYPE CommandLinkClicked(DWORD) override
    {
        return E_NOTIMPL;
    }
    HRESULT STDMETHODCALLTYPE GetUserSid(PWSTR *sid) override
    {
        return CopyText(metadata.sid, sid);
    }
    HRESULT STDMETHODCALLTYPE ReportResult(NTSTATUS, NTSTATUS, PWSTR *message,
                                           CREDENTIAL_PROVIDER_STATUS_ICON *icon) override
    {
        *message = nullptr;
        *icon = CPSI_NONE;
        return S_OK;
    }
    HRESULT STDMETHODCALLTYPE GetSerialization(CREDENTIAL_PROVIDER_GET_SERIALIZATION_RESPONSE *status,
                                               CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION *output, PWSTR *message,
                                               CREDENTIAL_PROVIDER_STATUS_ICON *icon) override
    {
        *status = CPGSR_NO_CREDENTIAL_NOT_FINISHED;
        ZeroMemory(output, sizeof(*output));
        *message = nullptr;
        *icon = CPSI_NONE;
        LoginRequest request{};
        request.version = 1;
        request.verb = 1;
        CopyMemory(request.device, metadata.device, sizeof(request.device));
        CopyMemory(request.instance, metadata.instance, sizeof(request.instance));
        CopyMemory(request.operation, metadata.operation, sizeof(request.operation));
        SecretResponse response;
        if (!Exchange(request, response.value) ||
            memcmp(response.value.device, metadata.device, sizeof(metadata.device)) ||
            memcmp(response.value.instance, metadata.instance, sizeof(metadata.instance)) ||
            memcmp(response.value.operation, metadata.operation, sizeof(metadata.operation)) ||
            wcscmp(response.value.sid, metadata.sid))
            return S_OK;
        HRESULT result = Serialize(response.value, output);
        if (SUCCEEDED(result))
            *status = CPGSR_RETURN_CREDENTIAL_FINISHED;
        return result;
    }
};

class Provider final : public ICredentialProvider, public ICredentialProviderSetUserArray
{
    LONG references = 1;
    ICredentialProviderEvents *events = nullptr;
    UINT_PTR context = 0;
    HWND window = nullptr;
    alignas(wchar_t) LoginResponse metadata{};
    bool available = false;
    static LRESULT CALLBACK WindowProc(HWND window, UINT message, WPARAM wparam, LPARAM lparam)
    {
        auto self = reinterpret_cast<Provider *>(GetWindowLongPtrW(window, GWLP_USERDATA));
        if (message == WM_NCCREATE)
        {
            self = static_cast<Provider *>(reinterpret_cast<CREATESTRUCTW *>(lparam)->lpCreateParams);
            SetWindowLongPtrW(window, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(self));
        }
        if (message == WM_TIMER && self)
        {
            self->AddRef();
            self->Refresh();
            self->Release();
            return 0;
        }
        return DefWindowProcW(window, message, wparam, lparam);
    }
    void Refresh()
    {
        LoginRequest request{};
        request.version = 1;
        SecretResponse response;
        bool ready = Exchange(request, response.value);
        bool changed = ready != available ||
                       (ready && memcmp(metadata.operation, response.value.operation, sizeof(metadata.operation)));
        available = ready;
        if (ready)
        {
            metadata = response.value;
            SecureZeroMemory(metadata.password, sizeof(metadata.password));
        }
        if (changed && events)
            events->CredentialsChanged(context);
    }

  public:
    Provider()
    {
        InterlockedIncrement(&objects);
    }
    ~Provider()
    {
        UnAdvise();
        UnregisterClassW(L"WCCS-Login-Refresh-v1", module);
        InterlockedDecrement(&objects);
    }
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID id, void **result) override
    {
        if (!result)
            return E_POINTER;
        *result = nullptr;
        if (id == IID_IUnknown || id == IID_ICredentialProvider)
            *result = static_cast<ICredentialProvider *>(this);
        else if (id == IID_ICredentialProviderSetUserArray)
            *result = static_cast<ICredentialProviderSetUserArray *>(this);
        else
            return E_NOINTERFACE;
        AddRef();
        return S_OK;
    }
    ULONG STDMETHODCALLTYPE AddRef() override
    {
        return InterlockedIncrement(&references);
    }
    ULONG STDMETHODCALLTYPE Release() override
    {
        ULONG count = InterlockedDecrement(&references);
        if (!count)
            delete this;
        return count;
    }
    HRESULT STDMETHODCALLTYPE SetUsageScenario(CREDENTIAL_PROVIDER_USAGE_SCENARIO usage, DWORD) override
    {
        return usage == CPUS_LOGON ? S_OK : E_NOTIMPL;
    }
    HRESULT STDMETHODCALLTYPE SetSerialization(const CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION *) override
    {
        return E_NOTIMPL;
    }
    HRESULT STDMETHODCALLTYPE SetUserArray(ICredentialProviderUserArray *) override
    {
        return S_OK;
    }
    HRESULT STDMETHODCALLTYPE Advise(ICredentialProviderEvents *callback, UINT_PTR value) override
    {
        UnAdvise();
        events = callback;
        events->AddRef();
        context = value;
        WNDCLASSW definition{};
        definition.lpfnWndProc = WindowProc;
        definition.hInstance = module;
        definition.lpszClassName = L"WCCS-Login-Refresh-v1";
        if (!RegisterClassW(&definition) && GetLastError() != ERROR_CLASS_ALREADY_EXISTS)
            return HRESULT_FROM_WIN32(GetLastError());
        window = CreateWindowExW(0, definition.lpszClassName, L"", 0, 0, 0, 0, 0, HWND_MESSAGE, nullptr, module, this);
        if (!window || !SetTimer(window, 1, 500, nullptr))
            return HRESULT_FROM_WIN32(GetLastError());
        return S_OK;
    }
    HRESULT STDMETHODCALLTYPE UnAdvise() override
    {
        if (window)
        {
            KillTimer(window, 1);
            DestroyWindow(window);
            window = nullptr;
        }
        if (events)
        {
            events->Release();
            events = nullptr;
        }
        available = false;
        return S_OK;
    }
    HRESULT STDMETHODCALLTYPE GetFieldDescriptorCount(DWORD *count) override
    {
        *count = 1;
        return S_OK;
    }
    HRESULT STDMETHODCALLTYPE GetFieldDescriptorAt(DWORD id, CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR **output) override
    {
        *output = nullptr;
        if (id != 0)
            return E_INVALIDARG;
        auto descriptor = static_cast<CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR *>(
            CoTaskMemAlloc(sizeof(CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR)));
        if (!descriptor)
            return E_OUTOFMEMORY;
        ZeroMemory(descriptor, sizeof(*descriptor));
        descriptor->cpft = CPFT_LARGE_TEXT;
        HRESULT result = CopyText(L"WCCS contest sign-in", &descriptor->pszLabel);
        if (FAILED(result))
        {
            CoTaskMemFree(descriptor);
            return result;
        }
        *output = descriptor;
        return S_OK;
    }
    HRESULT STDMETHODCALLTYPE GetCredentialCount(DWORD *count, DWORD *selected, BOOL *automatic) override
    {
        *count = available ? 1 : 0;
        *selected = available ? 0 : CREDENTIAL_PROVIDER_NO_DEFAULT;
        *automatic = available;
        return S_OK;
    }
    HRESULT STDMETHODCALLTYPE GetCredentialAt(DWORD id, ICredentialProviderCredential **output) override
    {
        *output = nullptr;
        if (!available || id != 0)
            return E_INVALIDARG;
        *output = new (std::nothrow) Credential(metadata);
        return *output ? S_OK : E_OUTOFMEMORY;
    }
};

class Factory final : public IClassFactory
{
    LONG references = 1;

  public:
    Factory()
    {
        InterlockedIncrement(&objects);
    }
    ~Factory()
    {
        InterlockedDecrement(&objects);
    }
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID id, void **output) override
    {
        *output = nullptr;
        if (id != IID_IUnknown && id != IID_IClassFactory)
            return E_NOINTERFACE;
        *output = this;
        AddRef();
        return S_OK;
    }
    ULONG STDMETHODCALLTYPE AddRef() override
    {
        return InterlockedIncrement(&references);
    }
    ULONG STDMETHODCALLTYPE Release() override
    {
        ULONG count = InterlockedDecrement(&references);
        if (!count)
            delete this;
        return count;
    }
    HRESULT STDMETHODCALLTYPE CreateInstance(IUnknown *outer, REFIID id, void **output) override
    {
        if (outer)
            return CLASS_E_NOAGGREGATION;
        auto provider = new (std::nothrow) Provider();
        if (!provider)
            return E_OUTOFMEMORY;
        HRESULT result = provider->QueryInterface(id, output);
        provider->Release();
        return result;
    }
    HRESULT STDMETHODCALLTYPE LockServer(BOOL lock) override
    {
        if (lock)
            InterlockedIncrement(&objects);
        else
            InterlockedDecrement(&objects);
        return S_OK;
    }
};
STDAPI DllGetClassObject(REFCLSID id, REFIID iid, void **output)
{
    *output = nullptr;
    if (id != ProviderId)
        return CLASS_E_CLASSNOTAVAILABLE;
    auto factory = new (std::nothrow) Factory();
    if (!factory)
        return E_OUTOFMEMORY;
    HRESULT result = factory->QueryInterface(iid, output);
    factory->Release();
    return result;
}
STDAPI DllCanUnloadNow()
{
    return objects == 0 ? S_OK : S_FALSE;
}
BOOL WINAPI DllMain(HINSTANCE instance, DWORD reason, LPVOID)
{
    if (reason == DLL_PROCESS_ATTACH)
    {
        module = instance;
        DisableThreadLibraryCalls(instance);
    }

    return TRUE;
}
