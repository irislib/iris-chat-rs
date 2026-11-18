// JXA entry point. Run only inside the assigned synthetic GUI session.
function run(args) {
    const [pid, action, argument] = args;
    const se = Application("System Events");
    const app = se.applicationProcesses.whose({unixId: Number(pid)})[0];
    if (!app.exists() || Number(app.unixId()) !== Number(pid)) throw Error("App PID unavailable");
    app.frontmost = true;
    function attr(element, name) {
        try { return String(element.attributes.byName(name).value() || ""); }
        catch (_) { return ""; }
    }
    function find(predicate) {
        const result = [];
        const walk = (element, depth) => {
            if (predicate(element)) result.push(element);
            if (depth < 28) {
                let children;
                try { children = element.uiElements(); } catch (_) { return; }
                for (const child of children) walk(child, depth + 1);
            }
        };
        for (const window of app.windows()) walk(window, 0);
        // A native popup menu may live directly under the application.
        for (const menu of app.menus()) walk(menu, 0);
        return result;
    }
    function one(predicate, description) {
        const until = Date.now() + 30000;
        do {
            const found = find(predicate);
            if (found.length > 1) throw Error("Duplicate " + description);
            if (found.length === 1) return found[0];
            delay(0.3);
        } while (Date.now() < until);
        throw Error("Missing " + description);
    }
    const element = id => one(el => attr(el, "AXIdentifier") === id, id);
    const label = el => [attr(el, "AXTitle"), attr(el, "AXDescription"), attr(el, "AXValue")].join(" ");
    const hasTitle = (el, titles) => ["AXTitle","AXDescription","AXValue"].some(k => titles.includes(attr(el,k)));
    const press = el => el.actions.byName("AXPress").perform();
    function choosePath(path, choosingFolder) {
        one(el => attr(el, "AXRole") === "AXButton" && hasTitle(el, ["Open","Choose","Select"]),
            "native selection panel");
        se.keystroke("g", {using:["command down", "shift down"]});
        one(el => ["AXTextField","AXComboBox"].includes(attr(el,"AXRole")) && attr(el,"AXFocused")==="true",
            "Go to Folder input");
        se.keystroke(path);
        se.keyCode(36);
        const confirm = one(el => attr(el,"AXRole")==="AXButton" && attr(el,"AXEnabled")==="true"
            && hasTitle(el, ["Open","Choose","Select"]), choosingFolder ? "Choose folder" : "Open file");
        press(confirm);
    }
    if (action === "window") {
        ObjC.import("CoreGraphics");
        if (!$.CGPreflightScreenCaptureAccess()) throw Error("No window capture permission");
        const windows = ObjC.deepUnwrap($.CGWindowListCopyWindowInfo(17, 0));
        const own = windows.filter(w => w.kCGWindowOwnerPID === Number(pid) && w.kCGWindowLayer === 0
            && w.kCGWindowBounds.Width > 200 && w.kCGWindowBounds.Height > 200);
        if (own.length !== 1) throw Error("Cannot identify one candidate window");
        return String(own[0].kCGWindowNumber);
    }
    if (action === "open-self") {
        const search = element("chatListSearchField");
        search.attributes.byName("AXFocused").value = true;
        se.keystroke("a", {using:"command down"});
        se.keystroke("note to self");
        press(one(el => attr(el,"AXRole")==="AXButton" && label(el).toLowerCase().includes("note to self"),
                  "Note to Self search result"));
        element("chatMessageInput");
    } else if (action === "send") {
        press(element("chatAttachButton"));
        press(one(el => attr(el,"AXIdentifier")==="chatDirectFileButton" ||
            (attr(el,"AXRole")==="AXMenuItem" && label(el).includes("Send directly")), "Send directly"));
        choosePath(argument, false);
        element("chatSelectedAttachments");
        element("chatDirectFileMode");
        press(one(el => attr(el, "AXIdentifier") === "chatSendButton" && attr(el, "AXEnabled") === "true", "enabled Send button"));
    } else if (action === "offer") {
        element("chatDirectTransfer-" + argument);
        element("chatDirectTransferAccept-" + argument);
        one(el => attr(el,"AXRole")==="AXStaticText" && hasTitle(el, ["Ready to receive"]), "Ready to receive");
    } else if (action === "accept") {
        press(element("chatDirectTransferAccept-" + argument));
        choosePath(args[3], true);
    } else if (action === "completed") {
        element("chatDirectTransfer-" + argument);
        element("chatDirectTransferOpen-" + argument + "-0");
        one(el => attr(el,"AXRole")==="AXStaticText" && hasTitle(el, ["Received"]), "Received");
    } else throw Error("Unknown UI operation");
    return "ok";
}
