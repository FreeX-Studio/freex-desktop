/*
 * Copyright (c) 2026 MirraNET, Niklas Linz. All rights reserved.
 *
 * This file is part of the MirraNET project and is licensed under the
 * GNU Lesser General Public License v3.0 (LGPLv3).
 *
 * You may use, distribute and modify this code under the terms
 * of the LGPLv3 license. You should have received a copy of the
 * license along with this file. If not, see <https://www.gnu.org/licenses/lgpl-3.0.html>
 * or contact: niklas.linz@mirranet.de
 */

/*
 * Runs in every page of the main window (the server picker and FreeX Web of each server) before
 * the page's own scripts. It makes the web view behave like an app window instead of a browser:
 * no browser context menu, no print/find/view-source shortcuts, F11 for full screen of the
 * window, Alt+Left/Right for back and forward, and no dragging of images and links out of the
 * window. Pages handle their own keys first; this only acts on keys a page left alone.
 *
 * While mpv plays inside the window (the app sets window.__FREEX_MPV_EMBEDDED__), the keyboard
 * still belongs to the web view, so every key goes to mpv instead of the page.
 */
(() => {
    "use strict";
    if (window.__FREEX_NATIVE__ || window.top !== window) return;
    window.__FREEX_NATIVE__ = true;

    const editable = (el) => !!el && el.nodeType === 1 &&
        (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName));

    function appWindow() {
        const tauri = window.__TAURI__;
        return tauri && tauri.window && tauri.window.getCurrentWindow ? tauri.window.getCurrentWindow() : null;
    }

    async function toggleFullscreen() {
        const win = appWindow();
        if (!win) return;
        try {
            if (document.fullscreenElement) {
                await document.exitFullscreen();
                return;
            }
            await win.setFullscreen(!(await win.isFullscreen()));
        } catch (e) {
            // not allowed on this page
        }
    }

    // The browser's menu (Back, Reload, Save as, Print, Inspect) does not belong in an app.
    // Text fields and selected text keep theirs for copy and paste.
    document.addEventListener("contextmenu", (e) => {
        if (e.defaultPrevented || editable(e.target)) return;
        if (String(window.getSelection() || "").trim()) return;
        e.preventDefault();
    });

    // DOM key names that mpv calls differently; letters, digits and F1..F12 are the same.
    const MPV_KEYS = {
        " ": "SPACE", "#": "SHARP", "ArrowLeft": "LEFT", "ArrowRight": "RIGHT", "ArrowUp": "UP",
        "ArrowDown": "DOWN", "Escape": "ESC", "Enter": "ENTER", "Backspace": "BS", "Tab": "TAB",
        "Delete": "DEL", "Insert": "INS", "Home": "HOME", "End": "END", "PageUp": "PGUP",
        "PageDown": "PGDWN", "MediaPlayPause": "PLAYPAUSE", "MediaStop": "STOP",
        "MediaTrackNext": "NEXT", "MediaTrackPrevious": "PREV", "AudioVolumeUp": "VOLUME_UP",
        "AudioVolumeDown": "VOLUME_DOWN", "AudioVolumeMute": "MUTE", "F11": "f",
    };

    /** The key of an event in mpv's notation (Ctrl+Shift+LEFT, f, F), or null to ignore. */
    function mpvKey(e) {
        if (["Control", "Shift", "Alt", "Meta", "AltGraph", "CapsLock", "Dead", "Unidentified"].includes(e.key)) return null;
        let name = MPV_KEYS[e.key] || e.key;
        const named = name.length > 1;
        let mods = "";
        if (e.ctrlKey) mods += "Ctrl+";
        if (e.altKey) mods += "Alt+";
        if (e.metaKey) mods += "Meta+";
        // A typed character already carries Shift (F, !); named keys need it spelled out.
        if (e.shiftKey && named) mods += "Shift+";
        return mods + name;
    }

    window.addEventListener("keydown", (e) => {
        if (!window.__FREEX_MPV_EMBEDDED__) return;
        const tauri = window.__TAURI__;
        const key = mpvKey(e);
        if (!key || !tauri || !tauri.core) return;
        e.preventDefault();
        e.stopImmediatePropagation();
        tauri.core.invoke("mpv_key", {key}).catch(() => null);
    }, true);

    // Ctrl+<key> shortcuts of a browser that make no sense here: print, find, find next, view
    // source, save, open file, downloads, history, new window.
    const BROWSER_KEYS = new Set(["p", "f", "g", "u", "s", "o", "j", "h", "n"]);

    window.addEventListener("keydown", (e) => {
        if (e.defaultPrevented) return;
        const ctrl = e.ctrlKey || e.metaKey;
        const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
        if (key === "F11") {
            e.preventDefault();
            toggleFullscreen();
        } else if (key === "Escape" && !document.fullscreenElement) {
            const win = appWindow();
            if (win) win.isFullscreen().then((fs) => fs && win.setFullscreen(false)).catch(() => null);
        } else if (e.altKey && !ctrl && (key === "ArrowLeft" || key === "ArrowRight") && !editable(e.target)) {
            e.preventDefault();
            if (key === "ArrowLeft") history.back(); else history.forward();
        } else if (ctrl && !e.altKey && BROWSER_KEYS.has(key)) {
            e.preventDefault();
        } else if (key === "F3" || key === "F7" || key === "F12" || (ctrl && e.shiftKey && (key === "i" || key === "j" || key === "c"))) {
            // find next, caret browsing, developer tools
            e.preventDefault();
        }
    });

    // Images and links stay in the window instead of being dragged out as files or URLs.
    document.addEventListener("dragstart", (e) => {
        const t = e.target;
        if (t && t.nodeType === 1 && !t.closest("[draggable='true']") && t.closest("img, a")) e.preventDefault();
    });
})();
