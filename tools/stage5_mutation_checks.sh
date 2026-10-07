#!/bin/bash
cd /home/corymaynard370/Bit-code
export CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/mnt/chromeos/removable/MOVESPEED/aetherforge-target
F=crates/aether-app/src/project/planfile/verify.rs
OUT=~/girder-evidence/s5-mutations.txt; : > $OUT
mut() { name=$1; old=$2; new=$3
  python3 - "$F" "$old" "$new" <<'PY'
import sys
p,old,new=sys.argv[1:4]; s=open(p).read()
assert s.count(old)==1,("pattern not unique",old,s.count(old)); open(p,"w").write(s.replace(old,new))
PY
  if [ $? -ne 0 ]; then echo "$name => PATTERN ERROR" >> $OUT; git checkout -q -- $F; return; fi
  res=$(cargo test -p aether-app --test verified_edits -j1 --quiet 2>&1 | grep -a -E "test result|^    \"[a-z0-9-]+:" | head -4 | cut -c1-160 | tr '\n' ' ')
  echo "$name => $res" >> $OUT
  git checkout -q -- $F
}
mut "M1 pre-apply-sibling-rule" "&& fingerprint(&other.path, &other.source) == *supplied" "&& false"
mut "M2 post-apply-wrong-overload" "if nx.is_some() && nx == ny {" "if false {"
mut "M3 edge-comparison" "if edges_added != declared.edges_added || edges_removed != declared.edges_removed {" "if false {"
mut "M4 baseline-check" "if *supplied == fingerprint(&node.path, &node.source) {" "if true {"
mut "M5 module-nodes-included" "        .filter(|n| n.kind != NodeKind::Module)
" ""
mut "M6 fingerprint-without-path" "    hasher.update(path.as_bytes());
" ""
echo "restored: $(git status --short | wc -l) modified" >> $OUT
echo DONE >> $OUT
