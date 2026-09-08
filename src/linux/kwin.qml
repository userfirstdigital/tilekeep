import QtQuick
import QtQuick.Window
import QtQml.Models
import org.kde.kwin
import org.kde.ksvg as KSvg

Item {
    id: root

    property int gap: __TILEKEEP_GAP__
    property bool paused: false
    readonly property bool dryRun: __TILEKEEP_DRY_RUN__
    readonly property real minRatio: 0.05
    readonly property real maxRatio: 0.95
    property var monitors: []
    property var focused: null
    property var floating: new Set()
    property var identities: new Map()
    property var expectedGeometry: new Map()
    property var deferredPlacements: new Map()
    property var interactiveWindows: new Set()
    property var placementQueue: []
    property var currentPlacement: null
    property int placementAttempt: 0
    property var windowConnections: new Map()
    property var previewOwner: null
    property var previewGeometry: null
    property var previewArea: null
    property string previewZone: "center"
    property int sequence: 1
    property var pendingSnapshot: null
    property var resizeGuides: []
    property bool displayTransition: false
    property int displayEpoch: 0
    property string displayFingerprint: ""
    property int displayStableTicks: 0
    property var displaySamples: []
    property var pendingWindows: new Set()
    property var deferredSnapshot: null
    property bool saveAfterDisplay: false

    function rect(r) { return {x: Math.round(r.x), y: Math.round(r.y), width: Math.round(r.width), height: Math.round(r.height)}; }
    function rectRight(r) { return r.x + r.width; }
    function rectBottom(r) { return r.y + r.height; }
    function area(r) { return r.width * r.height; }
    function contains(r, p) { return p.x >= r.x && p.y >= r.y && p.x < rectRight(r) && p.y < rectBottom(r); }
    function inset(r) { return {x:r.x+gap, y:r.y+gap, width:Math.max(0,r.width-2*gap), height:Math.max(0,r.height-2*gap)}; }
    function leaf() { return {kind:"leaf", windows:[], active:0, vacated:null, remembered:null, parent:null}; }
    function leaves(node, out) {
        out = out || [];
        if (node.kind === "leaf") out.push(node);
        else { leaves(node.first, out); leaves(node.second, out); }
        return out;
    }
    function allWindows(node) {
        const out=[];
        for (const slot of leaves(node)) for (const window of slot.windows) out.push(window);
        return out;
    }
    function slotOf(w) {
        for (const m of monitors) for (const s of leaves(m.root)) if (s.windows.includes(w)) return [m,s];
        return null;
    }
    function replaceNode(old, replacement, monitor) {
        const parent = old.parent;
        replacement.parent = parent;
        if (!parent) monitor.root = replacement;
        else if (parent.first === old) parent.first = replacement;
        else parent.second = replacement;
    }
    function splitSlot(monitor, target, axis, newFirst, w) {
        const added = leaf(); added.windows = [w];
        const available=rects(monitor).get(target), a=minimumSize(target), b=minimumSize(added);
        const needed=axis==="x"?{width:a.width+b.width+gap,height:Math.max(a.height,b.height)}:
            {width:Math.max(a.width,b.width),height:a.height+b.height+gap};
        // Do not create an impossible split and hope the client ignores its
        // minimum. Sharing the existing slot keeps both windows usable.
        if(needed.width>available.width||needed.height>available.height) { assign(target,w);return target; }
        const split = {kind:"split", axis, ratio:0.5, first:newFirst?added:target, second:newFirst?target:added, parent:null};
        const oldParent = target.parent;
        split.parent = oldParent; target.parent = split; added.parent = split;
        if (!oldParent) monitor.root = split;
        else if (oldParent.first === target) oldParent.first = split;
        else oldParent.second = split;
        return added;
    }
    function minimumSize(node,exclude) {
        if(node.kind==="leaf") {
            let width=0,height=0;
            for(const w of node.windows) {
                // Invisible windows keep their remembered slot, but must not
                // prevent a visible neighbor from using the free space.
                if(w===exclude||!visibleHere(w))continue;
                const min=w.minSize||{width:0,height:0},frame=w.frameGeometry,client=w.clientGeometry;
                const borderWidth=frame&&client?Math.max(0,frame.width-client.width):0;
                const borderHeight=frame&&client?Math.max(0,frame.height-client.height):0;
                width=Math.max(width,Math.ceil(min.width+borderWidth));
                height=Math.max(height,Math.ceil(min.height+borderHeight));
            }
            return {width,height};
        }
        const collapsed=collapsedSide(node);
        if(collapsed)return minimumSize(collapsed<0?node.second:node.first,exclude);
        if(vacantSubtree(node))return {width:0,height:0};
        const a=minimumSize(node.first,exclude),b=minimumSize(node.second,exclude);
        return node.axis==="x"?{width:a.width+b.width+gap,height:Math.max(a.height,b.height)}:
            {width:Math.max(a.width,b.width),height:a.height+b.height+gap};
    }
    function splitExtent(usable,ratio,firstMin,secondMin) {
        const preferred=Math.round(usable*ratio);
        return firstMin+secondMin<=usable?Math.max(firstMin,Math.min(usable-secondMin,preferred)):preferred;
    }
    function vacantSubtree(node) { return leaves(node).every(s=>vacantHere(s)); }
    function collapsedSide(node) {
        if(node.preserveSpace)return 0;
        const first=vacantSubtree(node.first),second=vacantSubtree(node.second);
        if(first&&!second&&node.ratio<=minRatio)return -1;
        if(second&&!first&&node.ratio>=maxRatio)return 1;
        return 0;
    }
    function compute(node, r, result) {
        // Keep internal rectangles too: resizing a nested split needs its bounds.
        result.set(node, r);
        if (node.kind === "leaf") return;
        // A resize against the old 5%/95% stop can leave a thin, unusable
        // vacancy. Give that space (and its extra gap) to the visible sibling.
        // Keep the leaf and its occupants so restoring a hidden window or
        // opening a replacement still has a remembered place in the tree.
        const collapsed=collapsedSide(node),collapseFirst=collapsed<0,collapseSecond=collapsed>0;
        if(collapseFirst||collapseSecond) {
            const empty=node.axis==="x"?
                {x:collapseFirst?r.x:rectRight(r),y:r.y,width:0,height:r.height}:
                {x:r.x,y:collapseFirst?r.y:rectBottom(r),width:r.width,height:0};
            compute(node.first,collapseFirst?empty:r,result);
            compute(node.second,collapseSecond?empty:r,result);
            return;
        }
        const a=minimumSize(node.first),b=minimumSize(node.second);
        if (node.axis === "x") {
            const usable = Math.max(0, r.width-gap), first = splitExtent(usable,node.ratio,a.width,b.width);
            compute(node.first,{x:r.x,y:r.y,width:first,height:r.height},result);
            compute(node.second,{x:r.x+first+gap,y:r.y,width:usable-first,height:r.height},result);
        } else {
            const usable = Math.max(0, r.height-gap), first = splitExtent(usable,node.ratio,a.height,b.height);
            compute(node.first,{x:r.x,y:r.y,width:r.width,height:first},result);
            compute(node.second,{x:r.x,y:r.y+first+gap,width:r.width,height:usable-first},result);
        }
    }
    function rects(m) { const out=new Map(); compute(m.root,inset(m.area),out); return out; }
    function windowRect(w) { return rect(w.frameGeometry); }
    function identity(w) { return String(w.desktopFileName || w.resourceClass || w.resourceName || "").toLowerCase(); }
    function tileable(w) {
        // The Xwayland remote-control permission UI is a normal window, not a
        // transient dialog. Keep compositor security prompts outside the layout.
        if(w&&identity(w)==="org.kde.kwin.eisprompter")return false;
        return w && w.managed && !w.deleted && w.normalWindow && !w.specialWindow && !w.transient &&
            w.moveable && w.resizeable && !w.skipTaskbar && String(w.caption || "").length > 0;
    }
    function visibleHere(w) {
        if (w.minimized || w.fullScreen || w.hidden || w.maximizeMode) return false;
        if (!w.onAllDesktops && w.desktops && !w.desktops.includes(Workspace.currentDesktop)) return false;
        if (w.activities && w.activities.length && !w.activities.includes(Workspace.currentActivity)) return false;
        return true;
    }
    function monitorForOutput(output) { return monitors.find(m => m.online!==false&&m.output===output) || monitors.find(m=>m.online!==false); }
    function readDisplayState() {
        const out=[];
        for(const output of Workspace.screens) {
            try {
                // KWin temporarily substitutes this synthetic 1920×1080 output
                // when the last physical connector disappears. Never tile it.
                const name=String(output.name||"");
                if(!name||/^Placeholder(?:-|$)/i.test(name)||output.placeholder===true||output.enabled===false)continue;
                const a=rect(Workspace.clientArea(KWin.MaximizeArea,output,Workspace.currentDesktop));
                if(![a.x,a.y,a.width,a.height].every(Number.isFinite)||a.width<=0||a.height<=0)continue;
                out.push({name,output,area:a});
            }catch(e) { /* Output QObjects can disappear during a hotplug batch. */ }
        }
        return out;
    }
    function sameDisplays(samples) {
        const active=monitors.filter(m=>m.online!==false);
        return active.length===samples.length&&samples.every(s=>active.some(m=>m.name===s.name&&m.output===s.output));
    }
    function displaysReady() {
        if(displayTransition)return false;
        if(Workspace.screens!==undefined&&!sameDisplays(readDisplayState())){beginDisplayTransition();return false;}
        return true;
    }
    function beginDisplayTransition() {
        if(!displayTransition)console.log("Tilekeep: display transition; preserving real-monitor layouts");
        displayTransition=true;displayEpoch++;displayStableTicks=0;displayFingerprint="";displaySamples=[];
        placementDeadline.stop();placementSpacing.stop();
        placementQueue=[];currentPlacement=null;expectedGeometry.clear();deferredPlacements.clear();
        interactiveWindows.clear();hidePreview();
    }
    function commitDisplays(samples) {
        // Keep offline roots and window membership. Do not re-enroll those
        // windows on another output just because KWin temporarily relocated them.
        for(const m of monitors){m.online=false;m.output=null;}
        for(const s of samples) {
            const m=monitors.find(m=>m.name===s.name);
            if(m){m.output=s.output;m.area=s.area;m.online=true;}
            else monitors.push({name:s.name,output:s.output,area:s.area,online:true,root:leaf()});
        }
    }
    function pollDisplays() {
        // The optional guard also lets pure geometry fixtures omit a workspace.
        if(Workspace.screens===undefined)return false;
        const samples=readDisplayState();
        if(!displayTransition&&!sameDisplays(samples))beginDisplayTransition();
        if(!displayTransition)return false;
        if(!samples.length) {
            displayStableTicks=0;displayFingerprint="";displaySamples=[];
            // Don't retain references to destroyed output objects while asleep.
            for(const m of monitors){m.online=false;m.output=null;}
            return true;
        }
        const fingerprint=JSON.stringify(samples.map(s=>[s.name,s.area.x,s.area.y,s.area.width,s.area.height]).sort((a,b)=>a[0].localeCompare(b[0])));
        const sameObjects=samples.length===displaySamples.length&&samples.every(s=>displaySamples.some(p=>p.name===s.name&&p.output===s.output));
        if(fingerprint!==displayFingerprint||!sameObjects){displayFingerprint=fingerprint;displayStableTicks=0;displaySamples=samples;return true;}
        // Four quiet 500ms intervals include the panel's late strut restoration.
        if(++displayStableTicks<4)return true;
        commitDisplays(samples);displayTransition=false;displaySamples=[];
        if(deferredSnapshot!==null){const saved=deferredSnapshot;deferredSnapshot=null;restoreSnapshot(saved);}
        for(const w of Array.from(pendingWindows)){pendingWindows.delete(w);if(tileable(w))appeared(w);}
        // Also discover clients opened before the runtime started with no outputs.
        for(const w of Workspace.stackingOrder||[])if(tileable(w))appeared(w);
        console.log("Tilekeep: display layout restored after stable work areas",samples.map(s=>s.name).join(", "));
        apply();
        if(saveAfterDisplay){saveAfterDisplay=false;saveSnapshot();}
        return true;
    }
    function refreshWorkAreas() {
        if(displayTransition)return false;
        let changed=false;
        for(const m of monitors) {
            if(m.online===false)continue;
            const a=rect(Workspace.clientArea(KWin.MaximizeArea,m.output,Workspace.currentDesktop)),old=m.area;
            if(a.x!==old.x||a.y!==old.y||a.width!==old.width||a.height!==old.height) {
                m.area=a;changed=true;
            }
        }
        return changed;
    }
    function workAreasChanged() {
        if(!enabled)return;
        if(pollDisplays()||interactiveWindows.size)return;
        if(refreshWorkAreas())apply();
    }
    function syncMonitors() {
        const samples=readDisplayState();
        if(!monitors.length&&samples.length&&!displayTransition){commitDisplays(samples);return;}
        if(!samples.length&&!displayTransition)beginDisplayTransition();
        if(!displayTransition&&!sameDisplays(samples))beginDisplayTransition();
        if(!displayTransition)refreshWorkAreas();
    }
    function assign(slot,w) { slot.windows.push(w); slot.active=slot.windows.length-1; slot.vacated=null; slot.remembered=null; }
    function appeared(w) {
        if(!tileable(w)||slotOf(w)||floating.has(w))return false;
        if(!displaysReady()||!monitors.some(m=>m.online!==false)){pendingWindows.add(w);return false;}
        if(pendingSnapshot&&restorePendingWindow(w))return true;
        const m=monitorForOutput(w.output); if (!m) return false;
        const id=identity(w); identities.set(w,id);
        // Visually free slots can still contain minimized/off-desktop windows.
        // Prefer usable vacancies before reopening collapsed placeholders or
        // splitting an occupied slot, even when focus points at a small stack.
        const added=leaf();added.windows=[w];const min=minimumSize(added);
        const candidates=[];
        for(const monitor of [m,...monitors.filter(other=>other!==m&&other.online!==false)]) {
            const bounds=rects(monitor);
            for(const slot of leaves(monitor.root)) {
                const r=bounds.get(slot);
                if(vacantHere(slot)&&r.width>=Math.max(1,min.width)&&r.height>=Math.max(1,min.height))
                    candidates.push({monitor,slot,r});
            }
        }
        candidates.sort((a,b)=>Number(b.monitor===m)-Number(a.monitor===m)||
            Number(b.slot.remembered===id)-Number(a.slot.remembered===id)||
            (b.slot.vacated||0)-(a.slot.vacated||0)||area(b.r)-area(a.r));
        if(candidates.length){assign(candidates[0].slot,w);return true;}
        const ls=leaves(m.root), remembered=ls.filter(s=>!s.windows.length&&s.remembered===id).sort((a,b)=>(b.vacated||0)-(a.vacated||0))[0];
        if (remembered) assign(remembered,w);
        else {
            const empty=ls.filter(s=>!s.windows.length).sort((a,b)=>(b.vacated||0)-(a.vacated||0))[0];
            if (empty) assign(empty,w);
            else {
                const map=rects(m),focus=focused&&slotOf(focused);
                const ordered=ls.slice().sort((a,b)=>Number(focus&&focus[0]===m&&b===focus[1])-Number(focus&&focus[0]===m&&a===focus[1])||area(map.get(b))-area(map.get(a)));
                let chosen=null;
                for(const target of ordered) {
                    const r=map.get(target),old=minimumSize(target),axes=r.width>=r.height?["x","y"]:["y","x"];
                    for(const axis of axes) {
                        const fits=axis==="x"?old.width+min.width+gap<=r.width&&Math.max(old.height,min.height)<=r.height:
                            Math.max(old.width,min.width)<=r.width&&old.height+min.height+gap<=r.height;
                        if(fits){chosen={target,axis};break;}
                    }
                    if(chosen)break;
                }
                const target=chosen?chosen.target:ls.slice().sort((a,b)=>area(map.get(b))-area(map.get(a)))[0];
                const r=map.get(target);splitSlot(m,target,chosen?chosen.axis:r.width>=r.height?"x":"y",false,w);
            }
        }
        console.log("Tilekeep: tracking",id,String(w.caption));
        return true;
    }
    function detach(w) {
        const found=slotOf(w); if (!found) return;
        const s=found[1], i=s.windows.indexOf(w); s.windows.splice(i,1);
        if (!s.windows.length) { s.active=0; s.vacated=sequence++; s.remembered=identities.get(w)||null; }
        else { if(i<s.active)s.active--; s.active=Math.min(s.active,s.windows.length-1); }
    }
    function vanished(w) {
        pendingWindows.delete(w);
        hidePreview(w);
        detach(w);floating.delete(w);identities.delete(w);expectedGeometry.delete(w);
        deferredPlacements.delete(w);interactiveWindows.delete(w);
        placementQueue=placementQueue.filter(p=>p.window!==w);
        if(currentPlacement&&currentPlacement.window===w)finishCurrentPlacement();
        if(focused===w)focused=null;
    }
    function placements() {
        const out=[];
        for (const m of monitors) {
            if(m.online===false)continue;
            const map=rects(m);
            for (const s of leaves(m.root)) {
                const stack=s.windows.filter(w=>visibleHere(w)).length>1;
                for(let i=0;i<s.windows.length;i++) out.push({window:s.windows[i],rect:map.get(s),output:m.output,active:stack&&i===s.active});
            }
        }
        return out.sort((a,b)=>Number(a.active)-Number(b.active));
    }
    function placeWindow(w,r) {
        if(!displaysReady())return;
        if(w.tile) w.tile.unmanage(w);
        w.setMaximize(false,false);
        const next=Object.assign({},w.frameGeometry);
        next.x=r.x;next.y=r.y;next.width=r.width;next.height=r.height;
        w.frameGeometry=next;
    }
    function apply() {
        if (!enabled || paused || !displaysReady() || interactiveWindows.size) return;
        refreshWorkAreas();
        const next=placements();
        placementDeadline.stop();placementSpacing.stop();
        placementQueue=[];currentPlacement=null;expectedGeometry.clear();deferredPlacements.clear();
        for(const p of next) {
            if (!visibleHere(p.window)) continue;
            if (dryRun) console.log("Tilekeep dry-run:",identity(p.window),JSON.stringify(p.rect));
            else {
                console.log("Tilekeep placement:",identity(p.window),String(p.window.caption),JSON.stringify(p.rect),"output",p.output.name);
                placementQueue.push(p);
            }
        }
        if(!dryRun) placeNextWindow();
    }
    function placeNextWindow() {
        if(!enabled||!displaysReady()||currentPlacement||!placementQueue.length)return;
        currentPlacement=placementQueue.shift();placementAttempt=0;
        const p=currentPlacement;
        if(!tileable(p.window)||!visibleHere(p.window)||floating.has(p.window)) { finishCurrentPlacement();return; }
        // Matching geometry does not mean exclusive ownership: a native KWin
        // tile can still couple this window to other windows during edge drags.
        if(p.window.tile)p.window.tile.unmanage(p.window);
        if(p.window.output===p.output&&geometryMatches(windowRect(p.window),p.rect)) { finishCurrentPlacement();return; }
        if(p.window.output!==p.output)Workspace.sendClientToScreen(p.window,p.output);
        expectedGeometry.set(p.window,p.rect);
        // A move can complete synchronously; arm the timer before sending it.
        placementDeadline.restart();
        placeWindow(p.window,p.rect);
    }
    function finishCurrentPlacement() {
        const p=currentPlacement;if(!p)return;
        placementDeadline.stop();
        expectedGeometry.delete(p.window);
        // Layout reconciliation must not activate an unrelated stack or steal
        // focus from a floating window/dialog. Explicit stack cycling activates.
        if(p.active&&focused===p.window&&tileable(p.window)&&visibleHere(p.window))Workspace.raiseWindow(p.window);
        currentPlacement=null;
        placementSpacing.restart();
    }
    function geometryMatches(got,wanted,tolerance) {
        const t=tolerance===undefined?2:tolerance;
        return got&&wanted&&Math.abs(got.x-wanted.x)<=t&&Math.abs(got.y-wanted.y)<=t&&Math.abs(got.width-wanted.width)<=t&&Math.abs(got.height-wanted.height)<=t;
    }
    function retryDeferredPlacements() {
        if(!enabled||dryRun||!displaysReady()||interactiveWindows.size||currentPlacement||placementQueue.length)return;
        for(const [w,p] of deferredPlacements) {
            if(tileable(w)&&visibleHere(w)&&!floating.has(w)&&slotOf(w)&&!geometryMatches(windowRect(w),p.rect))placementQueue.push(p);
        }
        deferredPlacements.clear();
        placeNextWindow();
    }
    function placementTimedOut() {
        if(!displaysReady())return;
        const p=currentPlacement;if(!p)return;
        if(!tileable(p.window)||!visibleHere(p.window)||floating.has(p.window)) { finishCurrentPlacement();return; }
        const got=windowRect(p.window);
        const wanted=expectedGeometry.get(p.window);
        if(!wanted||geometryMatches(got,wanted)) {
            finishCurrentPlacement();return;
        }
        if(placementAttempt<2) {
            placementAttempt++;placementDeadline.restart();placeWindow(p.window,wanted);return;
        }
        // Wayland clients can stop drawing while an output sleeps. A timeout is
        // not a refusal: retain the request and reconcile after repaint resumes.
        deferredPlacements.set(p.window,p);
        finishCurrentPlacement();
    }
    function distance(r,p) {
        const dx=p.x<r.x?r.x-p.x:p.x>=rectRight(r)?p.x-rectRight(r)+1:0;
        const dy=p.y<r.y?r.y-p.y:p.y>=rectBottom(r)?p.y-rectBottom(r)+1:0;
        return dx*dx+dy*dy;
    }
    function slotAt(p) {
        if(!displaysReady())return null;
        const m=monitors.find(m=>m.online!==false&&contains(m.area,p)); if(!m)return null;
        const map=rects(m), ls=leaves(m.root).filter(s=>area(map.get(s))>0), s=ls.find(s=>contains(map.get(s),p))||ls.sort((a,b)=>distance(map.get(a),p)-distance(map.get(b),p))[0];
        if(!s)return null;
        return [m,s,map.get(s)];
    }
    function zone(r,p) {
        const x=(p.x-r.x)/r.width,y=(p.y-r.y)/r.height;
        if(x>=.25&&x<=.75&&y>=.25&&y<=.75)return "center";
        return Math.abs(x-.5)>=Math.abs(y-.5)?(x<.5?"left":"right"):(y<.5?"top":"bottom");
    }
    function vacantHere(slot) { return !slot.windows.some(w=>visibleHere(w)); }
    function emptyZone(r,p) {
        const x=(p.x-r.x)/r.width,y=(p.y-r.y)/r.height;
        const horizontal=x<.25?"left":x>.75?"right":"";
        const vertical=y<.25?"top":y>.75?"bottom":"";
        return vertical&&horizontal?vertical+"-"+horizontal:horizontal||vertical||"center";
    }
    function emptyPart(r,z) {
        let result=Object.assign({},r);
        if(z.includes("left")||z.includes("right")) {
            const first=Math.round(Math.max(0,r.width-gap)/2);
            result.width=z.includes("left")?first:Math.max(0,r.width-gap)-first;
            if(z.includes("right"))result.x+=first+gap;
        }
        if(z.includes("top")||z.includes("bottom")) {
            const first=Math.round(Math.max(0,r.height-gap)/2);
            result.height=z.includes("top")?first:Math.max(0,r.height-gap)-first;
            if(z.includes("bottom"))result.y+=first+gap;
        }
        return result;
    }
    function fittingEmptyZone(w,r,z) {
        const added=leaf();added.windows=[w];const min=minimumSize(added);
        const fits=part=>part.width>=Math.max(1,min.width)&&part.height>=Math.max(1,min.height);
        if(!fits(r))return "unavailable";
        if(fits(emptyPart(r,z)))return z;
        // A quarter smaller than the app's minimum becomes a fitting half.
        if(z.includes("-"))for(const half of [z.split("-")[1],z.split("-")[0]])if(fits(emptyPart(r,half)))return half;
        return "center";
    }
    function partitionEmpty(m,target,z) {
        let selected=target;
        for(const axis of ["x","y"]) {
            const first=axis==="x"?z.includes("left"):z.includes("top");
            const second=axis==="x"?z.includes("right"):z.includes("bottom");
            if(!first&&!second)continue;
            const spare=leaf(),split={kind:"split",axis,ratio:.5,first:null,second:null,parent:null};
            replaceNode(selected,split,m);
            split.first=first?selected:spare;split.second=first?spare:selected;
            selected.parent=split;spare.parent=split;
        }
        return selected;
    }
    function dropPreview(w,hit,z) {
        const target=hit[1],r=hit[2],source=slotOf(w);
        if(vacantHere(target)) {
            // Simulate the tree edit as well as the fraction: vacating a tiny
            // source may collapse its old placeholder and change final bounds.
            function copy(n,parent) {
                const c=Object.assign({},n,{parent});
                if(n.kind==="leaf")c.windows=n.windows.slice();
                else {c.first=copy(n.first,c);c.second=copy(n.second,c);}return c;
            }
            const roots=monitors.map(m=>m.root),oldSequence=sequence;
            try {
                for(const m of monitors)m.root=copy(m.root,null);
                const p={x:r.x+r.width*(z.includes("left")?.1:z.includes("right")?.9:.5),
                    y:r.y+r.height*(z.includes("top")?.1:z.includes("bottom")?.9:.5)};
                if(!drop(w,p,false))return r;
                const found=slotOf(w);return rects(found[0]).get(found[1]);
            }finally {for(let i=0;i<monitors.length;i++)monitors[i].root=roots[i];sequence=oldSequence;}
        }
        if(z==="center"||(source&&source[1]===target&&target.windows.length<2))return r;
        const axis=z==="left"||z==="right"?"x":"y",newFirst=z==="left"||z==="top";
        const added=leaf();added.windows=[w];
        const a=minimumSize(target,w),b=minimumSize(added);
        const needed=axis==="x"?{width:a.width+b.width+gap,height:Math.max(a.height,b.height)}:
            {width:Math.max(a.width,b.width),height:a.height+b.height+gap};
        if(needed.width>r.width||needed.height>r.height)return r;
        const usable=(axis==="x"?r.width:r.height)-gap;
        const first=splitExtent(usable,.5,newFirst?b[axis==="x"?"width":"height"]:a[axis==="x"?"width":"height"],newFirst?a[axis==="x"?"width":"height"]:b[axis==="x"?"width":"height"]);
        const offset=newFirst?0:first+gap,extent=newFirst?first:usable-first;
        return axis==="x"?{x:r.x+offset,y:r.y,width:extent,height:r.height}:{x:r.x,y:r.y+offset,width:r.width,height:extent};
    }
    function drop(w,p,stack) {
        if(floating.has(w))return false;
        const hit=slotAt(p); if(!hit)return false;
        const [m,target,r]=hit,source=slotOf(w);
        if(vacantHere(target)) {
            const z=fittingEmptyZone(w,r,emptyZone(r,p));
            if(z==="unavailable")return false;
            // Retain hidden occupants in the source when possible. Never close
            // them or make them visible while allocating the empty target.
            const hidden=target.windows.slice(),active=target.active;
            detach(w);target.windows=[];target.active=0;
            if(source&&source[1]!==target)for(const hiddenWindow of hidden)assign(source[1],hiddenWindow);
            else {target.windows=hidden;target.active=active;}
            assign(partitionEmpty(m,target,z),w);return true;
        }
        const z=zone(r,p);
        if(source&&source[1]===target) {
            if(z==="center"||target.windows.length<2)return false;
            detach(w); splitSlot(m,target,z==="left"||z==="right"?"x":"y",z==="left"||z==="top",w); return true;
        }
        if(!target.windows.length) { detach(w); assign(target,w); return true; }
        if(z==="center"&&stack) { detach(w); assign(target,w); return true; }
        if(z==="center") {
            if(!source) { splitSlot(m,target,r.width>=r.height?"x":"y",false,w); return true; }
            const a=source[1],temp={windows:a.windows,active:a.active,vacated:a.vacated,remembered:a.remembered};
            a.windows=target.windows;a.active=target.active;a.vacated=target.vacated;a.remembered=target.remembered;
            target.windows=temp.windows;target.active=temp.active;target.vacated=temp.vacated;target.remembered=temp.remembered; return true;
        }
        detach(w); splitSlot(m,target,z==="left"||z==="right"?"x":"y",z==="left"||z==="top",w); return true;
    }
    function resizeEdges(before,after) {
        return ["left","right","top","bottom"].filter(e=>Math.abs(edgePosition(before,e)-edgePosition(after,e))>2);
    }
    function edgePosition(r,e) { return e==="left"?r.x:e==="right"?rectRight(r):e==="top"?r.y:rectBottom(r); }
    function setEdge(r,e,p) {
        if(e==="left"){r.width=rectRight(r)-p;r.x=p;}
        else if(e==="right")r.width=p-r.x;
        else if(e==="top"){r.height=rectBottom(r)-p;r.y=p;}
        else r.height=p-r.y;
    }
    function intersects(a,b,padding) {
        return a.x<rectRight(b)+padding&&rectRight(a)+padding>b.x&&a.y<rectBottom(b)+padding&&rectBottom(a)+padding>b.y;
    }
    // Recut only empty space. Occupied rectangles are constraints, not ratios
    // to be scaled when an unrelated ancestor divider moves.
    function layoutAround(items,bounds,depth) {
        if(!items.length)return leaf();
        if(depth>60)return null;
        if(items.length===1&&geometryMatches(items[0].rect,bounds,0))return Object.assign({},items[0].slot,{windows:items[0].slot.windows.slice(),parent:null});
        const cuts=[];
        for(const axis of ["x","y"]) {
            const start=bounds[axis],size=axis==="x"?bounds.width:bounds.height,end=start+size;
            const positions=new Set();
            for(const item of items){positions.add(item.rect[axis]-gap);positions.add(item.rect[axis]+(axis==="x"?item.rect.width:item.rect.height));}
            for(const p of positions) {
                if(p<start||p+gap>end||p===end||p+gap===start)continue;
                const first=[],second=[];
                for(const item of items) {
                    const lo=item.rect[axis],hi=lo+(axis==="x"?item.rect.width:item.rect.height);
                    if(hi<=p)first.push(item);else if(lo>=p+gap)second.push(item);else break;
                }
                if(first.length+second.length!==items.length)continue;
                cuts.push({axis,p,first,second,score:Math.abs(first.length-second.length)});
            }
        }
        cuts.sort((a,b)=>a.score-b.score);
        if(!cuts.length)return null;
        // Any unobstructed full cut is safe; no occupied rectangle is divided.
        const c=cuts[0],a=Object.assign({},bounds),b=Object.assign({},bounds);
        if(c.axis==="x"){a.width=c.p-bounds.x;b.x=c.p+gap;b.width=rectRight(bounds)-b.x;}
        else {a.height=c.p-bounds.y;b.y=c.p+gap;b.height=rectBottom(bounds)-b.y;}
        const first=layoutAround(c.first,a,depth+1),second=layoutAround(c.second,b,depth+1);
        if(!first||!second)return null;
        const s={kind:"split",axis:c.axis,ratio:(c.p-bounds[c.axis])/Math.max(1,(c.axis==="x"?bounds.width:bounds.height)-gap),preserveSpace:true,first,second,parent:null};
        first.parent=s;second.parent=s;return s;
    }
    function resizeLayout(w,before,after) {
        const found=slotOf(w);if(!found)return null;
        if(!displaysReady()||found[0].online===false)return null;
        const [m,slot]=found,map=rects(m),bounds=inset(m.area),target=rect(after),edges=resizeEdges(before,after);
        const items=leaves(m.root).filter(s=>s===slot||!vacantHere(s)).map(s=>({slot:s,rect:Object.assign({},s===slot?target:map.get(s))}));
        if(!edges.length)return null;
        // The work area, not the screen edge, is the outer limit (panels included).
        for(const e of edges) {
            const p=edgePosition(target,e),limit=edgePosition(bounds,e);
            setEdge(target,e,e==="left"||e==="top"?Math.max(limit,p):Math.min(limit,p));
        }
        // Only a directly adjacent window may share a resize. All other windows
        // remain fixed, even when they used to share an ancestor split.
        for(const e of edges) {
            const horizontal=e==="left"||e==="right",leading=e==="left"||e==="top";
            const opposite=e==="left"?"right":e==="right"?"left":e==="top"?"bottom":"top";
            for(const item of items) {
                if(item.slot===slot)continue;
                const r=item.rect,old=map.get(item.slot);
                const overlap=horizontal?Math.min(rectBottom(before),rectBottom(old))-Math.max(before.y,old.y):Math.min(rectRight(before),rectRight(old))-Math.max(before.x,old.x);
                const adjacent=overlap>0&&Math.abs(edgePosition(old,opposite)-edgePosition(before,e)-(leading?-gap:gap))<=2;
                if(!adjacent||!intersects(target,r,gap))continue;
                const min=minimumSize(item.slot),size=Math.max(1,horizontal?min.width:min.height);
                const limit=leading?edgePosition(old,e)+size+gap:edgePosition(old,e)-size-gap;
                setEdge(target,e,leading?Math.max(edgePosition(target,e),limit):Math.min(edgePosition(target,e),limit));
            }
            for(const item of items) {
                if(item.slot===slot)continue;
                const r=item.rect,old=map.get(item.slot);
                const overlap=horizontal?Math.min(rectBottom(before),rectBottom(old))-Math.max(before.y,old.y):Math.min(rectRight(before),rectRight(old))-Math.max(before.x,old.x);
                if(overlap>0&&Math.abs(edgePosition(old,opposite)-edgePosition(before,e)-(leading?-gap:gap))<=2&&intersects(target,r,gap))setEdge(r,opposite,edgePosition(target,e)+(leading?-gap:gap));
            }
        }
        items.find(i=>i.slot===slot).rect=target;
        for(let i=0;i<items.length;i++) {
            const item=items[i],min=minimumSize(item.slot),r=item.rect;
            if(r.width<Math.max(1,min.width)||r.height<Math.max(1,min.height))return null;
            for(let j=0;j<i;j++)if(intersects(r,items[j].rect,gap))return null;
        }
        const tree=layoutAround(items,bounds,0);if(!tree)return null;
        // Hidden windows keep a remembered home without constraining free space.
        const vacant=leaves(tree).filter(s=>!s.windows.length);
        for(const old of leaves(m.root).filter(s=>s!==slot&&vacantHere(s))) {
            if(!old.windows.length&&!old.remembered)continue;
            const home=vacant.shift()||leaves(tree).find(s=>s.windows.includes(w));
            for(const hidden of old.windows)home.windows.push(hidden);
            if(!home.remembered)home.remembered=old.remembered;
        }
        const check=new Map();compute(tree,bounds,check);
        for(const item of items) {
            const placed=leaves(tree).find(s=>s.windows.includes(item.slot.windows[0]));
            if(!placed||!geometryMatches(check.get(placed),item.rect,0))return null;
        }
        return {monitor:m,tree,rect:target};
    }
    function adjustRatio(w,before,after) {
        const plan=resizeLayout(w,before,after);if(!plan)return false;
        plan.monitor.root=plan.tree;return true;
    }
    function constrainedResize(w,before,after) {
        const plan=resizeLayout(w,before,after);if(plan)return plan.rect;
        // Stop at the last valid local layout instead of reverting the whole
        // gesture on release. Never move distant windows to force a fit.
        let low=0,high=1,result=Object.assign({},before);
        for(let i=0;i<12;i++) {
            const t=(low+high)/2,candidate={};
            for(const key of ["x","y","width","height"])candidate[key]=Math.round(before[key]+(after[key]-before[key])*t);
            const next=resizeLayout(w,before,candidate);
            if(next){low=t;result=next.rect;}else high=t;
        }
        return result;
    }
    function resizeSnap(w,before,after,edges) {
        const found=slotOf(w);if(!found)return {rect:after,guides:[]};
        const [m,slot]=found,bounds=inset(m.area),map=rects(m),result=rect(after),guides=[];
        const space=Object.assign({},bounds);
        for(const s of leaves(m.root))if(s!==slot&&!vacantHere(s)) {
            const r=map.get(s);
            if(Math.min(rectBottom(before),rectBottom(r))>Math.max(before.y,r.y)) {
                if(rectRight(r)<=before.x)setEdge(space,"left",Math.max(space.x,rectRight(r)+gap));
                if(r.x>=rectRight(before))setEdge(space,"right",Math.min(rectRight(space),r.x-gap));
            }
            if(Math.min(rectRight(before),rectRight(r))>Math.max(before.x,r.x)) {
                if(rectBottom(r)<=before.y)setEdge(space,"top",Math.max(space.y,rectBottom(r)+gap));
                if(r.y>=rectBottom(before))setEdge(space,"bottom",Math.min(rectBottom(space),r.y-gap));
            }
        }
        // No sticky state: six logical pixels to capture, seven to escape.
        for(const e of edges) {
            const horizontal=e==="left"||e==="right",start=horizontal?bounds.x:bounds.y,size=horizontal?bounds.width:bounds.height;
            const leading=e==="left"||e==="top",candidates=[{p:start,label:"Work area"},{p:start+size,label:"Work area"}];
            for(const f of [.25,.5,.75])candidates.push({p:Math.round(start+size*f),label:f===.5?"½":""+(f===.25?"¼":"¾")});
            for(const f of [.25,.5,.75])candidates.push({p:Math.round((horizontal?space.x:space.y)+(horizontal?space.width:space.height)*f),label:"Space fraction"});
            for(const s of leaves(m.root))if(s!==slot&&!vacantHere(s)) {
                const r=map.get(s),lo=horizontal?r.x:r.y,extent=horizontal?r.width:r.height;
                candidates.push({p:lo,label:"Aligned"},{p:lo+extent,label:"Aligned"},
                    {p:leading?lo+extent+gap:lo-gap,label:"Aligned"},
                    {p:leading?(horizontal?rectRight(result):rectBottom(result))-extent:(horizontal?result.x:result.y)+extent,label:"Equal size"});
            }
            const p=edgePosition(result,e),nearest=candidates.filter(c=>Math.abs(c.p-p)<=6).sort((a,b)=>Math.abs(a.p-p)-Math.abs(b.p-p))[0];
            if(nearest){setEdge(result,e,nearest.p);guides.push({edge:e,pos:nearest.p,label:nearest.label});}
        }
        // Never advertise a snap which the final constrained layout can't honor.
        const plan=guides.length?resizeLayout(w,before,result):null;
        if(!plan||!geometryMatches(plan.rect,result,0))return {rect:after,guides:[]};
        return {rect:result,guides};
    }
    function subtreeEmpty(node) { return leaves(node).every(s=>!s.windows.length); }
    function compactNode(node) {
        if(node.kind==="leaf")return node;
        node.first=compactNode(node.first);node.first.parent=node;
        node.second=compactNode(node.second);node.second.parent=node;
        if(subtreeEmpty(node.first))return node.second;
        if(subtreeEmpty(node.second))return node.first;
        return node;
    }
    function compact(m) { m.root=compactNode(m.root);m.root.parent=null; }
    function cycle(delta) {
        if(paused||!displaysReady())return;
        const f=focused&&slotOf(focused);if(!f||f[1].windows.length<2)return;
        const s=f[1],visible=s.windows.filter(w=>visibleHere(w));if(visible.length<2)return;
        const next=(visible.indexOf(focused)+delta+visible.length)%visible.length;
        focused=visible[next];s.active=s.windows.indexOf(focused);apply();Workspace.raiseWindow(focused);Workspace.activeWindow=focused;
    }
    function showPreview(w,r,full,z) {
        if(!enabled||paused||dryRun||!tileable(w)||floating.has(w))return;
        previewOwner=w;
        previewArea=full||null;previewZone=z||"center";
        const surface=full||r;
        const old=previewGeometry;
        if(!old||old.x!==r.x||old.y!==r.y||old.width!==r.width||old.height!==r.height) {
            previewGeometry=rect(r);
        }
        dragPreview.x=surface.x;dragPreview.y=surface.y;
        dragPreview.width=surface.width;dragPreview.height=surface.height;
        if(!dragPreview.visible)dragPreview.visible=true;
    }
    function hidePreview(w) {
        if(w&&previewOwner!==w)return;
        dragPreview.visible=false;previewOwner=null;previewGeometry=null;previewArea=null;
        resizeGuides=[];resizeGuide.visible=false;
    }
    function showResizeGuide(w,snap) {
        const found=slotOf(w);if(!found)return;
        const r=inset(found[0].area);
        resizeGuide.x=r.x;resizeGuide.y=r.y;resizeGuide.width=r.width;resizeGuide.height=r.height;
        resizeGuides=snap.guides;resizeGuide.visible=resizeGuides.length>0;previewOwner=w;
    }
    function connectWindow(w) {
        if(windowConnections.has(w))return;
        let drag=null;
        const handlers={};
        handlers.geometry=()=>{
            const wanted=expectedGeometry.get(w);if(!wanted)return;
            const got=windowRect(w);
            if(currentPlacement&&currentPlacement.window===w&&geometryMatches(got,wanted)) {
                placementDeadline.stop();finishCurrentPlacement();
            }
        };
        handlers.started=()=>{
            hidePreview();
            if(!displaysReady()){drag=null;return;}
            interactiveWindows.add(w);
            placementDeadline.stop();placementSpacing.stop();
            placementQueue=[];currentPlacement=null;expectedGeometry.clear();deferredPlacements.clear();
            if(!paused&&!displayTransition&&tileable(w)&&!floating.has(w))drag={epoch:displayEpoch,rect:windowRect(w),moving:w.move,cursor:Workspace.cursorPos?{x:Workspace.cursorPos.x,y:Workspace.cursorPos.y}:null,edges:[],raw:null};
        };
        handlers.stepped=g=>{
            if(!drag||paused||!displaysReady()||drag.epoch!==displayEpoch)return;
            if(!drag.moving) {
                const changed=resizeEdges(drag.rect,g);
                // Wayland's first step can still report the old committed frame.
                // Infer edge movement from the pointer too, so we don't send that
                // old size back and cancel the client's pending resize.
                if(drag.cursor&&Workspace.cursorPos)for(const e of ["left","right","top","bottom"]) {
                    const horizontal=e==="left"||e==="right",p=horizontal?drag.cursor.x:drag.cursor.y;
                    const delta=horizontal?Workspace.cursorPos.x-drag.cursor.x:Workspace.cursorPos.y-drag.cursor.y;
                    if(Math.abs(p-edgePosition(drag.rect,e))<=24&&Math.abs(delta)>2)changed.push(e);
                }
                drag.edges=Array.from(new Set(drag.edges.concat(changed)));
                if(!drag.edges.length)return;
                let raw=rect(g);
                if(drag.cursor&&Workspace.cursorPos) {
                    raw=Object.assign({},drag.rect);
                    for(const e of drag.edges)setEdge(raw,e,edgePosition(drag.rect,e)+(e==="left"||e==="right"?Workspace.cursorPos.x-drag.cursor.x:Workspace.cursorPos.y-drag.cursor.y));
                }
                drag.raw=raw;
                const bounded=constrainedResize(w,drag.rect,raw),snap=resizeSnap(w,drag.rect,bounded,drag.edges);
                showResizeGuide(w,snap);
                // The cursor remains free; each step is measured from its original
                // position, never from the last snapped client geometry.
                if(!geometryMatches(windowRect(w),snap.rect,0)&&!dryRun)w.frameGeometry=snap.rect;
                return;
            }
            const hit=slotAt(Workspace.cursorPos);if(!hit){hidePreview(w);return;}
            const empty=vacantHere(hit[1]);
            const z=empty?fittingEmptyZone(w,hit[2],emptyZone(hit[2],Workspace.cursorPos)):zone(hit[2],Workspace.cursorPos);
            const r=dropPreview(w,hit,z);
            showPreview(w,r,empty?hit[2]:null,z);
        };
        handlers.finished=()=>{
            interactiveWindows.delete(w);
            hidePreview(w);if(!drag||paused||!displaysReady()||drag.epoch!==displayEpoch){drag=null;apply();return;}
            const d=drag;drag=null;
            // KWin restores the starting geometry before emitting Finished when
            // Escape cancels a drag. The cursor can still be over another slot.
            if(d.moving) {
                if(!geometryMatches(windowRect(w),d.rect))drop(w,Workspace.cursorPos,false);
            } else {
                const final=windowRect(w);
                // Escape restores the initial geometry: do not commit the last guide.
                if(!geometryMatches(final,d.rect)) {
                    const bounded=constrainedResize(w,d.rect,d.raw||final);
                    const snap=resizeSnap(w,d.rect,bounded,d.edges.length?d.edges:resizeEdges(d.rect,final));
                    adjustRatio(w,d.rect,snap.rect);
                }
            }
            apply();
        };
        handlers.minimized=()=>{apply();};
        w.frameGeometryChanged.connect(handlers.geometry);
        w.interactiveMoveResizeStarted.connect(handlers.started);
        w.interactiveMoveResizeStepped.connect(handlers.stepped);
        w.interactiveMoveResizeFinished.connect(handlers.finished);
        w.minimizedChanged.connect(handlers.minimized);
        w.maximizedChanged.connect(handlers.minimized);
        w.fullScreenChanged.connect(handlers.minimized);
        w.desktopsChanged.connect(handlers.minimized);
        w.activitiesChanged.connect(handlers.minimized);
        windowConnections.set(w,handlers);
    }
    function disconnectWindow(w) {
        const h=windowConnections.get(w);if(!h)return;
        try { w.frameGeometryChanged.disconnect(h.geometry); } catch(e) {}
        try { w.interactiveMoveResizeStarted.disconnect(h.started); } catch(e) {}
        try { w.interactiveMoveResizeStepped.disconnect(h.stepped); } catch(e) {}
        try { w.interactiveMoveResizeFinished.disconnect(h.finished); } catch(e) {}
        try { w.minimizedChanged.disconnect(h.minimized); } catch(e) {}
        try { w.maximizedChanged.disconnect(h.minimized); } catch(e) {}
        try { w.fullScreenChanged.disconnect(h.minimized); } catch(e) {}
        try { w.desktopsChanged.disconnect(h.minimized); } catch(e) {}
        try { w.activitiesChanged.disconnect(h.minimized); } catch(e) {}
        windowConnections.delete(w);
    }
    function stackAtCursor() {
        if(paused)return;
        const w=Workspace.activeWindow,hit=slotAt(Workspace.cursorPos);if(!tileable(w)||floating.has(w)||!hit||hit[1].windows.includes(w))return;
        detach(w);assign(hit[1],w);focused=w;apply();
    }
    function unstack(w) {
        if(paused||!displaysReady()||interactiveWindows.size)return false;
        const found=slotOf(w);if(!found||found[1].windows.length<2)return false;
        detach(w);appeared(w);apply();return true;
    }
    function stop() {
        enabled=false;placementDeadline.stop();placementSpacing.stop();recoveryTimer.stop();workAreaTimer.stop();hidePreview();
        unloadCall.service="org.kde.KWin";
        unloadCall.path="/Scripting";
        unloadCall.method="unloadScript";
        unloadCall.arguments=["tilekeep-runtime"];
        unloadCall.call();
    }
    function setPaused(value) {
        paused=value;hidePreview();placementDeadline.stop();placementSpacing.stop();
        placementQueue=[];currentPlacement=null;expectedGeometry.clear();deferredPlacements.clear();
        if(!paused)apply();
    }
    function snapshotWindows() {
        return Workspace.stackingOrder.filter(w=>tileable(w)).map(w=>({token:String(w.internalId),app:identity(w),title:String(w.caption),pid:w.pid,rect:windowRect(w),floating:floating.has(w)}));
    }
    function snapshotTree(node) {
        if(node.kind==="leaf")return {kind:"leaf",windows:node.windows.map(w=>String(w.internalId)),active:node.active};
        return {kind:"split",axis:node.axis,ratio:node.ratio,preserveSpace:!!node.preserveSpace,first:snapshotTree(node.first),second:snapshotTree(node.second)};
    }
    function saveSnapshot() {
        if(!displaysReady()){saveAfterDisplay=true;return;}
        const snapshot={schema:1,gap,windows:snapshotWindows(),monitors:monitors.map(m=>({name:m.name,area:m.area,root:snapshotTree(m.root)}))};
        snapshotSave.arguments=[JSON.stringify(snapshot)];snapshotSave.call();
    }
    function restorePendingWindow(w) {
        if(!tileable(w)||slotOf(w))return false;
        const pending=pendingSnapshot;
        const item=pending.entries.find(e=>!e.window&&e.app.app===identity(w)&&e.app.title===String(w.caption))||
            pending.entries.find(e=>!e.window&&e.app.app===identity(w));
        if(!item)return false;
        item.window=w;identities.set(w,identity(w));floating.delete(w);
        if(item.slot)assign(item.slot,w);
        else {floating.add(w);if(visibleHere(w))placeWindow(w,item.app.rect);}
        snapshotRestored.arguments=[JSON.stringify({gap,missing:pending.entries.filter(e=>!e.window).length})];snapshotRestored.call();
        return true;
    }
    function restoreSnapshot(json) {
        if(displayTransition){deferredSnapshot=json;return;}
        if(interactiveWindows.size){console.log("Tilekeep: finish dragging before loading a snapshot");return;}
        const saved=JSON.parse(json),available=Workspace.stackingOrder.filter(w=>tileable(w)),used=new Set(),byToken=new Map();
        const entries=saved.windows.map(app=>{
            const w=available.find(w=>!used.has(w)&&identity(w)===app.app&&String(w.internalId)===app.token)||available.find(w=>!used.has(w)&&identity(w)===app.app&&String(w.caption)===app.title)||available.find(w=>!used.has(w)&&identity(w)===app.app);
            if(w)used.add(w);const e={app,window:w||null,slot:null};byToken.set(app.token,e);return e;
        });
        function tree(n,parent) {
            if(n.kind==="leaf") {
                const s=leaf();s.parent=parent;
                for(const token of n.windows){const e=byToken.get(token);if(!e)continue;e.slot=s;if(e.window)assign(s,e.window);}
                s.active=Math.min(n.active,Math.max(0,s.windows.length-1));return s;
            }
            const s={kind:"split",axis:n.axis,ratio:n.ratio,preserveSpace:!!n.preserveSpace,parent};s.first=tree(n.first,s);s.second=tree(n.second,s);return s;
        }
        syncMonitors();if(displayTransition){deferredSnapshot=json;return;}floating.clear();gap=saved.gap;
        const named=new Map(monitors.map(m=>[m,saved.monitors.find(s=>s.name===m.name)]));
        const usedMonitors=new Set(Array.from(named.values()).filter(Boolean));
        for(let i=0;i<monitors.length;i++) {
            const m=monitors[i],old=named.get(m)||(m.online!==false?saved.monitors.find(s=>!usedMonitors.has(s)):null);
            if(old)usedMonitors.add(old);
            m.root=old?tree(old.root,null):leaf();
        }
        // Extra windows are left open at their existing positions, outside the
        // restored layout. A missing saved app can claim its slot when it opens.
        for(const w of available)if(!slotOf(w))floating.add(w);
        for(const e of entries)if(e.window&&e.app.floating){floating.add(e.window);if(visibleHere(e.window))placeWindow(e.window,e.app.rect);}
        pendingSnapshot={entries};apply();
        snapshotRestored.arguments=[JSON.stringify({gap,missing:entries.filter(e=>!e.window).length})];snapshotRestored.call();
    }

    function adoptExistingGeometry() {
        // A runtime upgrade must not retile an already valid desktop. Rebuild
        // from actual frames, including user adjustments made while stopped.
        for(const m of monitors) {
            if(m.online===false)continue;
            const bounds=inset(m.area),windows=allWindows(m.root),items=[];
            for(const w of windows.filter(w=>visibleHere(w))) {
                const s=leaf();s.windows=[w];items.push({slot:s,rect:windowRect(w)});
            }
            if(!items.length)continue;
            if(items.some((item,i)=>item.rect.x<bounds.x||item.rect.y<bounds.y||rectRight(item.rect)>rectRight(bounds)||rectBottom(item.rect)>rectBottom(bounds)||items.slice(0,i).some(other=>intersects(item.rect,other.rect,gap))))continue;
            const tree=layoutAround(items,bounds,0);if(!tree)continue;
            const map=new Map();compute(tree,bounds,map);
            if(items.some(item=>{const s=leaves(tree).find(s=>s.windows.includes(item.slot.windows[0]));return !geometryMatches(map.get(s),item.rect,0);}))continue;
            const vacant=leaves(tree).filter(s=>!s.windows.length);
            for(const w of windows.filter(w=>!visibleHere(w)))assign(vacant.shift()||leaves(tree)[0],w);
            m.root=tree;
        }
    }
    function start() {
        syncMonitors();
        for(const w of Workspace.stackingOrder) { connectWindow(w); appeared(w); }
        adoptExistingGeometry();
        focused=Workspace.activeWindow; apply();
        console.log("Tilekeep: Plasma Wayland backend started; gap",gap,"dry-run",dryRun);
    }

    Component.onCompleted: start()
    Component.onDestruction: {
        placementDeadline.stop();placementSpacing.stop();recoveryTimer.stop();workAreaTimer.stop();hidePreview();
        for(const w of Array.from(windowConnections.keys())) disconnectWindow(w);
    }

    Connections {
        target: Workspace
        function onWindowAdded(w) { root.connectWindow(w);if(root.appeared(w))root.apply(); }
        function onWindowRemoved(w) { root.disconnectWindow(w);root.vanished(w);root.apply(); }
        function onWindowActivated(w) { if(w){if(root.appeared(w))root.apply();root.focused=w;const f=root.slotOf(w);if(f)f[1].active=f[1].windows.indexOf(w);} }
        function onScreensChanged() { root.beginDisplayTransition(); }
        function onCurrentDesktopChanged() { root.apply(); }
        function onCurrentActivityChanged() { root.apply(); }
    }

    Timer { id: placementDeadline; interval: 1000; repeat: false; onTriggered: root.placementTimedOut() }
    Timer { id: placementSpacing; interval: 100; repeat: false; onTriggered: root.placeNextWindow() }
    Timer { id: recoveryTimer; interval: 5000; running: !root.dryRun; repeat: true; onTriggered: root.retryDeferredPlacements() }
    // KWin's scripting API has no work-area-changed signal. Panel visibility,
    // position and size can change without screensChanged being emitted.
    Timer { id: workAreaTimer; interval: 500; running: true; repeat: true; onTriggered: root.workAreasChanged() }

    DBusCall { id: unloadCall }
    DBusCall { id: snapshotSave; service:"com.userfirst.Tilekeep";path:"/Tilekeep";dbusInterface:"com.userfirst.Tilekeep";method:"SaveSnapshot";onFailed:console.log("Tilekeep: snapshot save failed") }
    DBusCall { id: snapshotLoad; service:"com.userfirst.Tilekeep";path:"/Tilekeep";dbusInterface:"com.userfirst.Tilekeep";method:"ReadSnapshot";onFinished:(returnValue)=>root.restoreSnapshot(returnValue[0]);onFailed:console.log("Tilekeep: snapshot load failed") }
    DBusCall { id: snapshotRestored;service:"com.userfirst.Tilekeep";path:"/Tilekeep";dbusInterface:"com.userfirst.Tilekeep";method:"SnapshotRestored";onFailed:console.log("Tilekeep: snapshot status could not be saved") }

    // KWin hides its shared Workspace outline after emitting each drag step.
    // Keep our own surface alive, changing geometry only when the target changes.
    // The outline hint keeps it in KWin's overlay layer, out of application lists.
    Window {
        id: resizeGuide
        readonly property bool __kwin_outline: true
        transientParent: null
        flags: Qt.BypassWindowManagerHint | Qt.FramelessWindowHint |
            Qt.WindowTransparentForInput | Qt.WindowDoesNotAcceptFocus
        color: "transparent"
        visible: false
        Repeater {
            model: root.resizeGuides
            Rectangle {
                required property var modelData
                readonly property bool vertical: modelData.edge==="left"||modelData.edge==="right"
                x: vertical?modelData.pos-resizeGuide.x:0
                y: vertical?0:modelData.pos-resizeGuide.y
                width: vertical?1:resizeGuide.width
                height: vertical?resizeGuide.height:1
                color: "#998ac7ff"
            }
        }
    }
    Window {
        id: dragPreview
        readonly property bool __kwin_outline: true
        // The script's root Item is not in a visible application window.
        // Do not defer showing this surface until that implicit parent appears.
        transientParent: null
        flags: Qt.BypassWindowManagerHint | Qt.FramelessWindowHint |
            Qt.WindowTransparentForInput | Qt.WindowDoesNotAcceptFocus
        color: "transparent"
        visible: false
        Rectangle { anchors.fill: parent; color: root.previewArea?"#183b82f6":"transparent"; border.color: "#aa8ac7ff"; border.width: root.previewArea?2:0; radius: 5 }
        Rectangle { visible: !!root.previewArea; x: parent.width/2; width: 1; height: parent.height; color: "#778ac7ff" }
        Rectangle { visible: !!root.previewArea; y: parent.height/2; height: 1; width: parent.width; color: "#778ac7ff" }
        KSvg.FrameSvgItem {
            x: root.previewGeometry?root.previewGeometry.x-dragPreview.x:0
            y: root.previewGeometry?root.previewGeometry.y-dragPreview.y:0
            width: root.previewGeometry?root.previewGeometry.width:0
            height: root.previewGeometry?root.previewGeometry.height:0
            imagePath: "widgets/translucentbackground"
            Rectangle { anchors.fill: parent; color: root.previewZone==="unavailable"?"#30ff5555":"#304da6ff"; border.color: root.previewZone==="unavailable"?"#ccff8888":"#cc8ac7ff"; border.width: 2; radius: 5 }
        }
        Text {
            visible: !!root.previewArea
            anchors.horizontalCenter: parent.horizontalCenter; anchors.top: parent.top; anchors.topMargin: 12
            text: root.previewZone==="unavailable"?"Too small for this app":root.previewZone==="center"?"Full space":root.previewZone.includes("-")?"Quarter space":"Half space"
            color: "white"; style: Text.Outline; styleColor: "#203040"; font.pixelSize: 16
        }
    }

    ShortcutHandler { name: "TilekeepCompact"; text: "Tilekeep: compact monitor"; sequence: "Meta+Shift+K"; onActivated: { if(root.paused||!root.displaysReady())return;const m=root.monitorForOutput(Workspace.screenAt(Workspace.cursorPos)); if(m){root.compact(m);root.apply();} } }
    ShortcutHandler { name: "TilekeepRetile"; text: "Tilekeep: re-tile windows"; sequence: "Meta+Shift+L"; onActivated: { root.syncMonitors();root.apply(); } }
    ShortcutHandler { name: "TilekeepUnstack"; text: "Tilekeep: unstack active window"; onActivated: root.unstack(Workspace.activeWindow) }
    ShortcutHandler { name: "TilekeepFloat"; text: "Tilekeep: toggle floating"; sequence: "Meta+Shift+F"; onActivated: { if(root.paused||!root.displaysReady())return;const w=Workspace.activeWindow;if(!w)return;if(root.floating.has(w)){root.floating.delete(w);root.appeared(w);}else{root.detach(w);root.floating.add(w);}root.apply(); } }
    // A fresh action id avoids loading the old saved Meta+Shift+S binding,
    // which conflicts with Spectacle's default screenshot shortcut.
    ShortcutHandler { name: "TilekeepStackAtCursor"; text: "Tilekeep: stack at cursor"; sequence: "Meta+Shift+G"; onActivated: root.stackAtCursor() }
    ShortcutHandler { name: "TilekeepNext"; text: "Tilekeep: next stacked window"; sequence: "Meta+Shift+N"; onActivated: root.cycle(1) }
    ShortcutHandler { name: "TilekeepPrevious"; text: "Tilekeep: previous stacked window"; sequence: "Meta+Shift+B"; onActivated: root.cycle(-1) }
    ShortcutHandler { name: "TilekeepQuit"; text: "Tilekeep: quit"; sequence: "Meta+Shift+Q"; onActivated: root.stop() }
    ShortcutHandler { name: "TilekeepPause"; text: "Tilekeep: pause tiling"; onActivated: root.setPaused(true) }
    ShortcutHandler { name: "TilekeepResume"; text: "Tilekeep: resume tiling"; onActivated: root.setPaused(false) }
    ShortcutHandler {name:"TilekeepSaveSnapshot";text:"Tilekeep: save snapshot";onActivated:root.saveSnapshot()}
    ShortcutHandler {name:"TilekeepLoadSnapshot";text:"Tilekeep: load snapshot";onActivated:{snapshotLoad.arguments=[JSON.stringify(root.snapshotWindows())];snapshotLoad.call();}}
    Instantiator {
        model: 65
        delegate: ShortcutHandler {
            required property int index
            name: "TilekeepSetGap" + index
            text: "Tilekeep: set gap to " + index
            onActivated: { root.gap=index;root.apply(); }
        }
    }
}
