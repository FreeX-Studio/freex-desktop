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
 * FreeX Desktop: the server picker. Shown on start (and via "Switch server"); connects to the
 * last server right away when that is switched on, otherwise lists the known servers and lets
 * the user add one. Everything after that is FreeX Web, loaded from the server itself.
 */
(() => {
    "use strict";
    const invoke = window.__TAURI__.core.invoke;
    const $ = (id) => document.getElementById(id);

    // ------------------------------------------------------------------ texts

    const DE = {
        "Cancel": "Abbrechen",
        "Choose a server": "Server auswählen",
        "Add a server": "Server hinzufügen",
        "Connect": "Verbinden",
        "The address of your FreeX Media Server, for example 192.168.1.10, nas:32500 or https://media.example.org.":
            "Die Adresse deines FreeX Media Servers, zum Beispiel 192.168.1.10, nas:32500 oder https://media.example.org.",
        "App settings": "App-Einstellungen",
        "Connect to the last server on start": "Beim Start mit dem letzten Server verbinden",
        "Play videos with": "Videos abspielen mit",
        "FreeX player": "FreeX-Player",
        "mpv (plays every file directly)": "mpv (spielt jede Datei direkt ab)",
        "Start mpv in full screen": "mpv im Vollbild starten",
        "Connecting to {0} …": "Verbinde mit {0} …",
        "Pick one of your servers or add a new one.": "Wähle einen deiner Server oder füge einen neuen hinzu.",
        "Add your FreeX Media Server to get started.": "Füge deinen FreeX Media Server hinzu, um loszulegen.",
        "No FreeX Media Server answers at this address.": "Unter dieser Adresse antwortet kein FreeX Media Server.",
        "{0} is not reachable. Is the server running?": "{0} ist nicht erreichbar. Läuft der Server?",
        "This address is not valid.": "Diese Adresse ist ungültig.",
        "Remove": "Entfernen",
        "Remove {0} from the list?": "{0} aus der Liste entfernen?",
        "Remove {0}": "{0} entfernen",
        "Play inside the FreeX window": "Im FreeX-Fenster abspielen",
        "Found on this PC: {0}": "Auf diesem PC gefunden: {0}",
        "Last used {0}": "Zuletzt verwendet {0}",
        "The FreeX player plays in the app and converts files your PC cannot play on the server.":
            "Der FreeX-Player spielt in der App und lässt Dateien, die dein PC nicht abspielen kann, vom Server umwandeln.",
        "mpv plays every file directly, without conversion on the server. Skip intro and Watch Together are only available in the FreeX player.":
            "mpv spielt jede Datei direkt ab, ohne Umwandlung auf dem Server. Intro überspringen und Watch Together gibt es nur im FreeX-Player.",
        "mpv was not found. Reinstall FreeX Desktop or install mpv and add it to the PATH.":
            "mpv wurde nicht gefunden. Installiere FreeX Desktop neu oder installiere mpv und nimm es in den PATH auf.",
    };
    const german = (navigator.language || "").toLowerCase().startsWith("de");
    document.documentElement.lang = german ? "de" : "en";
    const t = (text, ...args) => (german && DE[text] || text).replace(/\{(\d)}/g, (_, i) => args[i]);
    document.querySelectorAll("[data-t]").forEach((el) => el.textContent = t(el.dataset.t));
    $("address").placeholder = german ? "z. B. 192.168.1.10" : "e.g. 192.168.1.10";

    const ERRORS = {
        unreachable: "No FreeX Media Server answers at this address.",
        invalid: "This address is not valid.",
        empty: "This address is not valid.",
    };
    const errorText = (e) => t(ERRORS[e] || String(e));

    function show(section) {
        for (const id of ["connecting", "choose"]) $(id).hidden = id !== section;
        $("settings").hidden = section !== "choose";
    }

    function showError(message) {
        $("error").textContent = message || "";
        $("error").hidden = !message;
    }

    // ------------------------------------------------------------------ connecting

    let attempt = 0;

    /** Checks the server and opens it; returns to the list with a message if that fails. */
    async function connect(address, label) {
        const mine = ++attempt;
        $("connecting-title").textContent = t("Connecting to {0} …", label || address);
        $("connecting-url").textContent = label ? address : "";
        show("connecting");
        try {
            const probe = await invoke("server_probe", {address});
            if (mine !== attempt) return;
            await invoke("server_open", {url: probe.url, name: probe.name});
        } catch (e) {
            if (mine !== attempt) return;
            await renderList();
            showError(e === "unreachable" && label ? t("{0} is not reachable. Is the server running?", label) : errorText(e));
            show("choose");
        }
    }

    $("cancel").addEventListener("click", async () => {
        attempt++;
        await renderList();
        show("choose");
    });

    $("add").addEventListener("submit", (e) => {
        e.preventDefault();
        const address = $("address").value.trim();
        if (address) connect(address);
    });

    // ------------------------------------------------------------------ list

    const relative = (ms) => {
        const days = Math.round((Date.now() - ms) / 86400000);
        try {
            return new Intl.RelativeTimeFormat(german ? "de" : "en", {numeric: "auto"}).format(-days, "day");
        } catch (e) {
            return new Date(ms).toLocaleDateString();
        }
    };

    function el(tag, cls, text) {
        const e = document.createElement(tag);
        if (cls) e.className = cls;
        if (text != null) e.textContent = text;
        return e;
    }

    let known = [];

    async function renderList() {
        const list = await invoke("servers_list");
        known = list.servers;
        $("auto-connect").checked = list.autoConnect;
        $("choose-intro").textContent = known.length
            ? t("Pick one of your servers or add a new one.")
            : t("Add your FreeX Media Server to get started.");
        const items = known.map((s) => {
            const open = el("button", "server");
            open.type = "button";
            open.append(el("span", "server-icon", (s.name || "?").slice(0, 1).toUpperCase()));
            const text = el("span", "server-text");
            text.append(el("span", "server-name", s.name), el("span", "server-url", s.url));
            open.append(text);
            if (s.lastUsed) open.append(el("span", "server-when", t("Last used {0}", relative(s.lastUsed))));
            open.addEventListener("click", () => connect(s.url, s.name));

            const remove = el("button", "remove", "×");
            remove.type = "button";
            remove.title = t("Remove {0}", s.name);
            remove.setAttribute("aria-label", t("Remove {0}", s.name));
            const li = el("li");
            // Asks in place instead of a browser dialog.
            remove.addEventListener("click", () => {
                const ask = el("div", "confirm");
                ask.append(el("span", null, t("Remove {0} from the list?", s.name)));
                const yes = el("button", "btn danger", t("Remove"));
                yes.type = "button";
                yes.addEventListener("click", async () => {
                    await invoke("server_remove", {id: s.id});
                    renderList();
                });
                const no = el("button", "btn", t("Cancel"));
                no.type = "button";
                no.addEventListener("click", () => li.replaceChildren(open, remove));
                ask.append(yes, no);
                li.replaceChildren(ask);
                no.focus();
            });
            li.append(open, remove);
            return li;
        });
        $("servers").replaceChildren(...items);
        $("servers").hidden = !items.length;
        return list;
    }

    /** A server installed on this PC is offered with one click. */
    async function lookLocally() {
        try {
            const probe = await invoke("server_probe", {address: "localhost"});
            if (known.some((s) => s.url === probe.url || s.url.replace("127.0.0.1", "localhost") === probe.url)) return;
            const b = $("found");
            b.textContent = t("Found on this PC: {0}", probe.name);
            b.onclick = () => invoke("server_open", {url: probe.url, name: probe.name}).catch((e) => showError(errorText(e)));
            b.hidden = false;
        } catch (e) {
            // no server on this PC
        }
    }

    // ------------------------------------------------------------------ settings

    let info = null;

    function renderPlayer() {
        const player = $("player");
        player.value = info.player;
        player.querySelector('option[value="mpv"]').disabled = !info.mpvAvailable && info.player !== "mpv";
        $("mpv-fullscreen").checked = info.mpvFullscreen;
        $("mpv-embed").checked = info.mpvEmbed;
        $("mpv-fs-row").hidden = info.player !== "mpv";
        // Wayland cannot show mpv inside another window; mpv then always opens its own.
        $("mpv-embed-row").hidden = info.player !== "mpv" || !info.mpvCanEmbed;
        $("player-hint").textContent = info.player === "mpv"
            ? (info.mpvAvailable ? t("mpv plays every file directly, without conversion on the server. Skip intro and Watch Together are only available in the FreeX player.")
                : t("mpv was not found. Reinstall FreeX Desktop or install mpv and add it to the PATH."))
            : t("The FreeX player plays in the app and converts files your PC cannot play on the server.");
    }

    $("player").addEventListener("change", async (e) => {
        info.player = e.target.value;
        await invoke("set_player", {player: info.player});
        renderPlayer();
    });
    $("mpv-fullscreen").addEventListener("change", async (e) => {
        info.mpvFullscreen = e.target.checked;
        await invoke("set_player", {player: info.player, mpvFullscreen: info.mpvFullscreen});
    });
    $("mpv-embed").addEventListener("change", async (e) => {
        info.mpvEmbed = e.target.checked;
        await invoke("set_player", {player: info.player, mpvEmbed: info.mpvEmbed});
    });
    $("auto-connect").addEventListener("change", (e) => invoke("set_auto_connect", {enabled: e.target.checked}));

    // ------------------------------------------------------------------ start

    (async () => {
        info = await invoke("desktop_info");
        $("version").textContent = `${info.app} ${info.version}`;
        renderPlayer();
        const list = await renderList();
        const params = new URLSearchParams(location.search);
        const picking = params.has("pick");
        // From the tray menu: switch to this server at once.
        const wanted = known.find((s) => s.id === params.get("connect"));
        if (wanted) {
            connect(wanted.url, wanted.name);
            return;
        }
        const last = known.find((s) => s.id === list.lastServer);
        if (!picking && list.autoConnect && last) {
            connect(last.url, last.name);
            return;
        }
        show("choose");
        if (!known.length) $("address").focus();
        lookLocally();
    })();
})();
