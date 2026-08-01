# @hhushhas/skillbox

The npm launcher provides the native Skillbox CLI through `npx` or a global npm install. It downloads the matching GitHub release binary on first use, verifies its SHA-256 checksum, caches it locally, and forwards all arguments to Skillbox.

```bash
npx @hhushhas/skillbox search "browser automation"
npx @hhushhas/skillbox setup --status
npm install --global @hhushhas/skillbox
skillbox list
```

The unscoped npm name `skillbox` belongs to another project, so this package uses the `@hhushhas/skillbox` scope.
