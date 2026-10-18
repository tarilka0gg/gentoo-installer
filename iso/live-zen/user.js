// The live session's Zen profile: made from this file every time Zen starts (get-zen): less RAM and CPU, less tracking, no first-run pages,
// and the author's Zen layout (compact mode, tabs on the right) with the ten Zen mods of zen-themes.json (Zen downloads them from the mod
// store at its first start).

// Less RAM and CPU: fewer processes, no site isolation, a small memory cache and no disk cache (the profile is in RAM anyway).
user_pref("fission.autostart", false);
user_pref("dom.ipc.processCount", 2);
user_pref("dom.ipc.processCount.webIsolated", 1);
user_pref("browser.tabs.unloadOnLowMemory", true);
user_pref("browser.cache.disk.enable", false);
user_pref("browser.cache.memory.enable", true);
user_pref("browser.cache.memory.capacity", 65536);
user_pref("browser.sessionstore.interval", 60000);
user_pref("general.smoothScroll", false);
user_pref("network.prefetch-next", false);
user_pref("network.dns.disablePrefetch", true);
user_pref("network.predictor.enabled", false);

// Less tracking and no reports.
user_pref("privacy.donottrackheader.enabled", true);
user_pref("browser.contentblocking.category", "strict");
user_pref("datareporting.healthreport.uploadEnabled", false);
user_pref("toolkit.telemetry.enabled", false);
user_pref("browser.newtabpage.activity-stream.feeds.telemetry", false);
user_pref("browser.newtabpage.activity-stream.telemetry", false);

// No first-run pages: straight to a browser.
user_pref("browser.startup.homepage_override.mstone", "ignore");
user_pref("startup.homepage_welcome_url", "");
user_pref("browser.aboutwelcome.enabled", false);
user_pref("zen.welcome-screen.seen", true);

// The author's layout: compact mode on from the start, tabs on the right, no "new tab" button at the top; unload idle tabs after 5 minutes.
user_pref("zen.tabs.vertical.right-side", true);
user_pref("zen.view.compact.enable-at-startup", true);
user_pref("zen.view.show-newtab-button-top", false);
user_pref("zen.tab-unloader.timeout-minutes", 5);
// Settings of the "Smaller Compact Mode" mod.
user_pref("theme.smaller_compact_mode.sidebar_height", "70");
