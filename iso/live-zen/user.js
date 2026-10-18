// The live session's Zen profile: made from this file every time Zen starts (get-zen). Nothing of the author's own Zen layout is in it
// (no compact mode, tab side, mods): only less RAM and CPU, less tracking, and no first-run pages.

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
