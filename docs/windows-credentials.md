# Windows scene credentials

Windows scenes can keep named values in the current user's Credential Manager.
This uses generic credentials, scoped by the scene/plugin owner and persisted
on this computer. It does not read browser passwords or unrelated credentials.
The operating system protects storage; this is not isolation from other native
programs running as the same user.

Declare `credentials.*` in `permissions.services`. Prefer asynchronous calls:

- `credentials.list`: returns names only, as a list of strings.
- `credentials.set(name, value)`: creates or replaces a case-insensitive name.
- `credentials.remove(name)`: removes it; an already absent name is success.

Names contain 1–60 characters without controls. Values contain 1–2560 UTF-8
bytes without NUL. Invalid arguments and OS failures return errors. No query
returns a saved value to Luau. The scene name is the storage identity, as with
`files`; renaming a scene does not migrate its credentials automatically.

With `desktop.*` permission, `desktop.type_secret(epoch, window_id, name)` reads
the scene-owned credential directly inside the native input service. The usual
catalog, fresh screenshot, foreground, cancellation, modifier and target checks
apply. It never places the value on the clipboard or in process arguments.
Native credential/readback and input buffers are wiped when released. The
initial value supplied by the scene still exists in the scene's input memory;
clear that field after a successful save.

Validation uses dummy values under unique test owners and removes them:

```powershell
cargo test --locked --lib platform::windows_credentials::tests::native_scoped_credential_roundtrip -- --ignored --exact
```

The native roundtrip covers Unicode names/values, owner separation,
case-insensitive lookup, overwrite and deletion. It does not establish typing
acceptance in a real password field. API semantics follow Microsoft's
[CredWriteW](https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credwritew)
and [CREDENTIALW](https://learn.microsoft.com/en-us/windows/win32/api/wincred/ns-wincred-credentialw)
documentation.
