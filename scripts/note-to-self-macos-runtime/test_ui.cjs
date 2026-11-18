// Recorded-control contract only; macOS Accessibility and real dialogs are live gates.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");
const source = fs.readFileSync(__dirname + "/mac_ui.js", "utf8");

function drive(action, value, flags = {}) {
    const log = [];
    const states = {};
    let view;
    let menu = [];
    const el = (id, role, title = "", press = () => {}, extra = {}) => ({
        attributes: {byName: k => ({value: () => ({AXIdentifier:id, AXRole:role, AXTitle:title,
                    AXEnabled:"true", ...extra})[k] || ""})},
        actions: {byName: () => ({perform: () => { log.push(id || title); press(); }})},
        uiElements: () => []
    });
    const button = (id, press) => el(id, "AXButton", "", press);
    const win = () => ({attributes: {byName: () => ({value: () => ""})}, uiElements: () => view});
    const app = {exists:()=>true, unixId:()=>1234, windows:()=>[win()], menus:()=>menu};
    states.search = [el("chatListSearchField","AXTextField"),
        el("", "AXButton", "Note to self", () => {view = states.composer;})];
    states.composer = [el("chatMessageInput","AXTextArea"),button("chatAttachButton", () => {
        menu = [el("chatDirectFileButton", "AXMenuItem", "Send directly", () => {
            menu = []; view = states.panel;
        })];
    })];
    states.ready = [el("chatSelectedAttachments", "AXScrollArea"), el("chatDirectFileMode","AXStaticText"),
        button("chatSendButton", () => {view = states.sent;})];
    states.sent = [];
    states.offer = [el("chatDirectTransfer-t","AXGroup"), el("", "AXStaticText", "Ready to receive"),
        button("chatDirectTransferAccept-t", () => {view = states.panel;})];
    states.completed = [el("chatDirectTransfer-t","AXGroup"), el("", "AXStaticText", "Received"), el("chatDirectTransferOpen-t-0","AXButton")];
    states.panel = [el("", "AXButton", "Open", () => {view = action === "send" ? states.ready : states.completed;})];
    states.goto = [el("", "AXTextField", "", undefined, {AXFocused:"true"})];
    view = action === "open-self" ? states.search : action === "send" ? states.composer
        : action === "completed" ? states.completed : states.offer;
    if (flags.duplicate) view = view.concat(view[0]);
    const system = {applicationProcesses:{whose:()=>[app]}, keystroke:(key) => {
        log.push("key "+key); if(key === "g") view = states.goto;
    }, keyCode:(key) => {log.push("code "+key); view = states.panel;}};
    const context = vm.createContext({Application:()=>system, delay:()=>{}, args:["1234",action,value,"/folder"]});
    const result = vm.runInContext(source+"\nrun(args)",context,{timeout:3000});
    return {log, result};
}
assert.deepEqual(drive("open-self","").log, ["key a","key note to self","Note to self"]);
assert.deepEqual(drive("send","/selected.bin").log,
    ["chatAttachButton","chatDirectFileButton","key g","key /selected.bin","code 36","Open","chatSendButton"]);
assert.deepEqual(drive("accept","t").log, ["chatDirectTransferAccept-t","key g","key /folder","code 36","Open"]);
assert.equal(drive("offer","t").result,"ok");
assert.equal(drive("completed","t").result,"ok");
assert.throws(()=>drive("offer","t",{duplicate:true}),/Duplicate/);
console.log("6 UI driver contract checks passed (simulated controls).");
