#!/usr/bin/env python3
"""Drive an owned Bevy template through real MCP and save GPU PNG evidence."""
import argparse
import base64
import json
from pathlib import Path
import queue
import subprocess
import threading
from PIL import Image, ImageChops


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary",type=Path,required=True)
    parser.add_argument("--out",type=Path,required=True)
    parser.add_argument("--software",action="store_true")
    args=parser.parse_args();args.out.mkdir(parents=True,exist_ok=True)
    messages=queue.Queue();log=(args.out/"game-stderr.log").open("w")
    command=[str(args.binary.resolve()),"--mcp"]+(["--software"] if args.software else [])
    child=subprocess.Popen(command,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=log,text=True,bufsize=1)
    def reader():
        try:
            for line in child.stdout:
                messages.put(json.loads(line))
        except Exception as error:messages.put(error)
        finally:messages.put(None)
    threading.Thread(target=reader,daemon=True).start();ordinal=0;transcript=[]
    def send(value):child.stdin.write(json.dumps(value)+"\n");child.stdin.flush()
    def request(method,params):
        nonlocal ordinal
        ordinal+=1;send({"jsonrpc":"2.0","id":ordinal,"method":method,"params":params})
        while True:
            value=messages.get(timeout=130)
            if value is None:raise RuntimeError(f"game exited early: {child.poll()}; see {args.out/'game-stderr.log'}")
            if isinstance(value,Exception):raise value
            if value.get("id")==ordinal:
                if "error" in value:raise RuntimeError(value["error"])
                return value["result"]
    def tool(name,arguments):
        value=request("tools/call",{"name":name,"arguments":arguments})
        assert not value.get("isError"),value
        transcript.append({"name":name,"arguments":arguments,"result":{k:v for k,v in value.items() if k!="content"}})
        return value
    def structured(value):
        return value.get("structuredContent") or json.loads(next(c["text"] for c in value["content"] if c["type"]=="text"))
    def screenshot(name,target="screen"):
        value=tool("game_screenshot",{"target":target});image=next(c for c in value["content"] if c["type"]=="image")
        assert image["mimeType"]=="image/png";path=args.out/name;path.write_bytes(base64.b64decode(image["data"],validate=True));return Image.open(path).convert("RGB")
    try:
        request("initialize",{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"template-smoke","version":"1"}})
        send({"jsonrpc":"2.0","method":"notifications/initialized","params":{}})
        names={tool["name"] for tool in request("tools/list",{})["tools"]}
        assert names=={"game_status","game_screenshot","game_step","game_set_input","game_pause"}
        initial=structured(tool("game_status",{}));assert initial["frame"]==0 and initial["paused"]
        before=screenshot("before.png");assert before.size==(320,224);assert len(before.getcolors(320*224))>2
        step=structured(tool("game_step",{"frames":20,"buttons":["right","b"]}));assert step["frame"]==20 and step["advanced_frames"]==20
        after=screenshot("after.png");assert ImageChops.difference(before,after).getbbox() is not None
        repeated=screenshot("after-repeat.png");assert after.tobytes()==repeated.tobytes(),"paused GPU screen must be stable"
        status=structured(tool("game_status",{}));assert status["frame"]==20 and status["paused"]
        release=structured(tool("game_step",{"frames":1,"buttons":[]}));assert release["frame"]==21
        assert structured(tool("game_status",{}))["controller_buttons"]==[]
        window=screenshot("window.png","window");assert window.width>=320 and window.height>=224
        tool("game_set_input",{"buttons":["a"]});tool("game_set_input",{"buttons":[]})
        tool("game_pause",{"paused":True})
        summary={"ok":True,"renderer":status.get("renderer"),"screen":list(before.size),"window":list(window.size),"exact_step_frames":20,"paused_capture_repeat_equal":True,"software_requested":args.software}
        (args.out/"summary.json").write_text(json.dumps(summary,indent=2)+"\n")
        (args.out/"transcript.json").write_text(json.dumps(transcript,indent=2)+"\n")
        child.stdin.close();child.wait(timeout=20);assert child.returncode==0
        print(json.dumps(summary))
    finally:
        if child.poll() is None:child.kill();child.wait()
        log.close()


if __name__=="__main__":main()
