// SPDX-License-Identifier: AGPL-3.0-or-later
// The normal DOMjudge clock resumes after a page reload following a state change.
(() => {
    const nativeClock = window.updateClock;
    let control = null;
    let revision = null;
    window.updateClock = function () {
        if (control?.paused && document.getElementById("timeleft")) {
            const seconds = Math.max(0, Math.floor(control.remaining_seconds));
            const hours = Math.floor(seconds / 3600);
            const minutes = Math.floor(seconds / 60) % 60;
            const remainder = seconds % 60;
            document.getElementById("timeleft").textContent =
                `paused · ${hours}:${String(minutes).padStart(2,"0")}:${String(remainder).padStart(2,"0")}`;
        } else {
            nativeClock();
        }
    };
    let pending = false;
    async function refresh() {
        if (pending || !window.wccsControlUrl) return;
        pending = true;
        try {
            const response = await fetch(window.wccsControlUrl, {cache:"no-store"});
            if (!response.ok) return;
            const value = await response.json();
            if (revision !== null && revision !== value.revision) {
                location.reload();
                return;
            }
            revision = value.revision;
            control = value;
            window.updateClock();
        } catch {
            // Preserve the last confirmed paused clock during a network outage.
        } finally {
            pending = false;
        }
    }
    void refresh();
    setInterval(() => void refresh(), 2000);
})();
