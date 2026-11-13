// Locally bundled xterm owns only this surface's child DOM. Rust owns all SSH,
// session identities and queues. No remote scripts, websockets or CDN requests.
(() => {
    let engine = null, scheduled = false, token = 0, pending = null, queue = [];
    function dispatch() {
        if (!engine) return;
        const source=document.querySelector(".terminal-state");
        let metadata;try{metadata=JSON.parse(source.textContent);}catch(_){return;}
        if(pending && metadata.input_ack>=pending.token)pending=null;
        if(!pending)pending=queue.shift()||null;
        if(!pending)return;
        if(pending.epoch!==metadata.epoch||pending.session!==metadata.id){pending=null;queue=[];return;}
        const input=document.querySelector(".terminal-input-bridge");
        if(!input)return;
        input.value=JSON.stringify(pending);
        input.dispatchEvent(new Event("input",{bubbles:true}));
    }
    function emit(value) {
        if(!engine)return;
        const previous=queue[queue.length-1];
        if(value.kind==="data"&&previous?.kind==="data"&&previous.id===value.id&&previous.epoch===engine.epoch&&previous.data.length+value.data.length<=2048){previous.data+=value.data;return;}
        queue.push({...value,epoch:engine.epoch,session:engine.id,token:++token});
        if(!pending)dispatch();
    }
    setInterval(()=>{if(pending||queue.length)dispatch();},80);
    function data(text, id) {
        // Do not split surrogate pairs or exceed the native input packet budget.
        let chunk = "";
        for (const character of text) {
            if (chunk.length + character.length > 2048) { emit({kind:"data",id,data:chunk}); chunk=""; }
            chunk += character;
        }
        if (chunk) emit({kind:"data",id,data:chunk});
    }
    function fit() {
        if (engine?.surface.getClientRects().length) {
            try { engine.fit.fit(); } catch (_) { /* zero-sized view during layout */ }
        }
    }
    function make(metadata, surface) {
        pending=null;queue=[];
        if (engine) { engine.resize.disconnect(); engine.term.dispose(); engine.wrapper.remove(); }
        const wrapper = document.createElement("div");
        wrapper.className="terminal-owned-surface";
        surface.appendChild(wrapper);
        const term = new Terminal({fontFamily:'"Cascadia Mono",Consolas,"Liberation Mono",monospace',fontSize:13,cursorBlink:true,scrollback:5000,convertEol:false,screenReaderMode:true,allowProposedApi:false,disableStdin:true,linkHandler:{activate:()=>{}}});
        const addon = new FitAddon.FitAddon();
        term.loadAddon(addon);
        term.open(wrapper);
        term.parser.registerOscHandler(52,()=>true); // Remote output cannot write the clipboard.
        const id=metadata.id, epoch=metadata.epoch,key=`${epoch}:${id}`;
        engine={id,epoch,key,term,fit:addon,wrapper,surface,lastPacket:0,clear:metadata.clear,paste:0,visible:false,status:null,selected:false};
        engine.resize=new ResizeObserver(()=>{fit();});
        engine.resize.observe(surface);
        term.onData(text=>{if(engine?.key===key && engine.status==="running") data(text,id);});
        term.onBinary(text=>{if(engine?.key===key && engine.status==="running") emit({kind:"binary",id,data:btoa(text)});});
        term.onResize(size=>emit({kind:"resize",id,columns:size.cols,rows:size.rows}));
        term.onSelectionChange(()=>{
            if (engine?.key!==key) return;
            const selected=term.hasSelection();
            if(selected!==engine.selected){engine.selected=selected;emit({kind:"selection",id,selected});}
        });
        term.attachCustomKeyEventHandler(event=>{
            if(event.type!=="keydown") return true;
            const chord=event.ctrlKey||event.metaKey;
            if(chord && event.key.toLowerCase()==="c" && (event.shiftKey||term.hasSelection())) {
                event.preventDefault();copy();return false;
            }
            if(chord && event.shiftKey && event.key.toLowerCase()==="v") {
                event.preventDefault();emit({kind:"paste",id});return false;
            }
            return true;
        });
        // Escape, Ctrl+K etc. belong to the shell, not Glory's app shortcuts.
        for(const name of ["keydown","keypress","keyup","input","paste"])
            wrapper.addEventListener(name,event=>event.stopPropagation());
        fit();
        emit({kind:"ready",columns:term.cols,rows:term.rows});
    }
    function copy() {
        if(engine?.term.hasSelection()) emit({kind:"copy",id:engine.id,text:engine.term.getSelection()});
    }
    document.addEventListener("click",event=>{
        if(!(event.target instanceof Element))return;
        const copyButton=event.target.closest(".terminal-copy"),pasteButton=event.target.closest(".terminal-paste");
        if(copyButton && !copyButton.disabled){event.preventDefault();event.stopImmediatePropagation();copy();engine?.term.focus();}
        if(pasteButton && !pasteButton.disabled && engine){event.preventDefault();event.stopImmediatePropagation();emit({kind:"paste",id:engine.id});engine.term.focus();}
    },true);
    function scan() {
        scheduled=false;
        const source=document.querySelector(".terminal-state"),surface=document.querySelector(".terminal-surface");
        if(!source||!surface)return;
        let metadata;
        try{metadata=JSON.parse(source.textContent);}catch(_){return;}
        if(!metadata.id){
            if(engine){engine.resize.disconnect();engine.term.dispose();engine.wrapper.remove();engine=null;}
            pending=null;queue=[];
            return;
        }
        const visible=!!surface.getClientRects().length;
        if(!engine && !visible)return;
        if(engine?.key!==`${metadata.epoch}:${metadata.id}`)make(metadata,surface);
        if(engine.surface!==surface){engine.resize.disconnect();engine.surface=surface;surface.appendChild(engine.wrapper);engine.resize.observe(surface);}
        const term=engine.term;
        const app=document.querySelector(".app");
        if(app){
            const style=getComputedStyle(app),background=style.getPropertyValue("--surface").trim(),foreground=style.getPropertyValue("--text").trim(),cursor=style.getPropertyValue("--accent").trim();
            const themeKey=`${background}:${foreground}:${cursor}`;
            if(engine.theme!==themeKey){engine.theme=themeKey;term.options.theme={background,foreground,cursor,selectionBackground:"#649cff66"};}
        }
        if(engine.status!==metadata.status){
            engine.status=metadata.status;
            term.options.disableStdin=metadata.status!=="running";
            if(metadata.status==="running"){fit();emit({kind:"resize",id:metadata.id,columns:term.cols,rows:term.rows});}
        }
        if(visible&&!engine.visible){fit();term.focus();}
        engine.visible=visible;
        if(engine.clear!==metadata.clear){engine.clear=metadata.clear;term.clear();}
        if(metadata.paste_revision>engine.paste){
            engine.paste=metadata.paste_revision;
            if(metadata.paste&&metadata.status==="running")term.paste(metadata.paste);
            emit({kind:"pasted",id:metadata.id,revision:metadata.paste_revision});
        }
        for(const element of document.querySelectorAll(".terminal-packet")){
            let packet;try{packet=JSON.parse(element.textContent);}catch(_){continue;}
            if(packet.id<=engine.lastPacket)continue;
            engine.lastPacket=packet.id;
            const bytes=Uint8Array.from(atob(packet.data),c=>c.charCodeAt(0));
            const id=metadata.id,key=engine.key;
            term.write(bytes,()=>{if(engine?.key===key)emit({kind:"consumed",id,packet:packet.id});});
        }
        if(pending&&metadata.input_ack>=pending.token){pending=null;dispatch();}
    }
    const schedule=()=>{if(!scheduled){scheduled=true;requestAnimationFrame(scan);}};
    new MutationObserver(records=>{
        if(records.some(record=>!(record.target instanceof Element ? record.target : record.target.parentElement)?.closest(".xterm")))schedule();
    }).observe(document,{subtree:true,childList:true,characterData:true,attributes:true,attributeFilter:["class"]});
    window.addEventListener("resize",schedule);
    schedule();
})();
