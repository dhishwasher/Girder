import json, sys, urllib.request
M = sys.argv[1]
S = json.load(open("/dev/stdin")) if False else None
SCHEMA = {"type":"object","required":["plan_version","steps"],"properties":{
 "plan_version":{"type":"integer"},
 "steps":{"type":"array","items":{"type":"object","required":["id","edits"],
  "properties":{"id":{"type":"string"},
   "edits":{"type":"array","items":{"type":"object",
    "required":["node","replace_node"],"properties":{
     "node":{"type":"string"},"replace_node":{"type":"string"}}}}}}}}}
P = ('Emit a Bit Code plan as JSON only.\n'
     'plan_version must be 2. Exactly one step with id "s1".\n'
     'One edit replacing node crate::sample::calc::greet with:\n'
     'def greet(name):\n    return f"hi {name}"\n')
for i in (1,2,3):
    b = json.dumps({"model":M,"messages":[{"role":"user","content":P}],
      "stream":False,"format":SCHEMA,"keep_alive":0,
      "options":{"temperature":0,"seed":42+i,"num_predict":400}}).encode()
    r = urllib.request.Request("http://127.0.0.1:11434/api/chat", b,
        {"Content-Type":"application/json"})
    try:
        d = json.load(urllib.request.urlopen(r, timeout=900))
        t = d["message"]["content"]
        print(f"attempt {i}: {t[:300]}")
    except Exception as e:
        print(f"attempt {i}: {e}")
