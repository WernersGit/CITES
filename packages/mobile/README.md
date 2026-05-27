# mobile

Entry point for the iOS and Android build. The mobile target uses the
same router and views as desktop and web, minus the `Analysis` page
which is desktop-only.

## Run it

```
dx serve --package mobile --platform ios
dx serve --package mobile --platform android
```

### iOS prerequisites

You need a full Xcode install (not just the CLT) and an accepted
license:

```
sudo xcodebuild -license
sudo xcode-select -s /Applications/Xcode.app/Contents/Developer
```

If `dx` fails with `No devices are booted` or an `FBSOpenApplication`
error, boot a simulator first:

```
xcrun simctl list devices available
xcrun simctl boot "<DEVICE-UDID>"
```

Then re-run `dx serve --package mobile --platform ios`.

## Features

- `mobile` (default): the actual app build.
- `server`: only used when the fullstack build needs SSR.
