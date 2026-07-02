#!/usr/bin/env bash
# #475 SCOPE probe suite — map every nested-vector shape across the axes:
# inner width · container context · construction path · access shape · nesting ·
# other collections · scale. Each probe prints one deterministic line; the runner
# classifies PASS / WRONG / CRASH on --interpret AND --native. Run against a given
# loft to map the true scope. (Cells are the probes; graduate survivors to files.)
LOFT="${LOFT:-target/debug/loft}"
D="$TMPDIR/scope475"; mkdir -p "$D"
NAMES=(); EXPECT=(); CLUSTER=()
cell() { local n="$1"; cat > "$D/$n.loft"; NAMES+=("$n"); EXPECT+=("$2"); CLUSTER+=("$3"); }
classify() { if [ "$1" -eq 124 ]; then echo TIMEOUT; elif [ "$1" -ne 0 ]; then echo "CRASH($1)"; elif [ "$2" = "$3" ]; then echo PASS; else echo "WRONG[$2]"; fi; }

# ---- Cluster A: inner scalar width (local, append, n=300, sum adj[j][0]=Σ0..299=44850) ----
for t in integer:0:44850 float:0.0:44850 single:0.0:44850 character:x:300 boolean:t:300 text:s:300; do
  ty="${t%%:*}"; rest="${t#*:}"; exp="${rest#*:}"
  case $ty in
   integer) mk='adj[j] += [j];'; rd='s += adj[j][0];'; init='s=0';;
   float)   mk='adj[j] += [j as float];'; rd='s += adj[j][0] as integer;'; init='s=0';;
   single)  mk='adj[j] += [j as single];'; rd='s += adj[j][0] as integer;'; init='s=0';;
   character) mk="adj[j] += ['x'];"; rd="if adj[j][0]=='x' { s += 1; }"; init='s=0';;
   boolean) mk='adj[j] += [true];'; rd='if adj[j][0] { s += 1; }'; init='s=0';;
   text)    mk='adj[j] += ["v"];'; rd='if adj[j][0]=="v" { s += 1; }'; init='s=0';;
  esac
  cell "A-inner-$ty" "$exp" A <<EOF
fn main() { n=300; adj: vector<vector<$ty>> = [];
  for i in 0..n { r: vector<$ty>=[]; adj += [r]; }
  for j in 0..n { $mk }
  $init; for j in 0..n { $rd } println("{s}"); }
EOF
done

# ---- Cluster B: container context ----
cell B-local '300 44850' B <<'EOF'
fn main() { n=300; adj: vector<vector<integer>> = [];
  for i in 0..n { r: vector<integer>=[]; adj += [r]; } for j in 0..n { adj[j] += [j]; }
  t=0; s=0; for j in 0..n { t+=len(adj[j]); s+=adj[j][0]; } println("{t} {s}"); }
EOF
cell B-struct-field '300 44850' B <<'EOF'
struct G { adj: vector<vector<integer>> }
fn main() { n=300; g = G { adj: [] };
  for i in 0..n { r: vector<integer>=[]; g.adj += [r]; } for j in 0..n { g.adj[j] += [j]; }
  t=0; s=0; for j in 0..n { t+=len(g.adj[j]); s+=g.adj[j][0]; } println("{t} {s}"); }
EOF
cell B-struct-ref '300 44850' B <<'EOF'
struct G { adj: vector<vector<integer>> }
fn add(g: &G, i: integer, x: integer) { g.adj[i] += [x]; }
fn main() { n=300; g = G { adj: [] };
  for i in 0..n { r: vector<integer>=[]; g.adj += [r]; } for j in 0..n { add(g,j,j); }
  t=0; s=0; for j in 0..n { t+=len(g.adj[j]); s+=g.adj[j][0]; } println("{t} {s}"); }
EOF
cell B-return '2 1 2' B <<'EOF'
fn make() -> vector<vector<integer>> { z: vector<vector<integer>> = [[1],[2]]; z }
fn main() { d = make(); println("{len(d)} {d[0][0]} {d[1][0]}"); }
EOF
cell B-reassign '2 1 2' B <<'EOF'
fn main() { z: vector<vector<integer>> = [[9]]; z = [[1],[2]]; println("{len(z)} {z[0][0]} {z[1][0]}"); }
EOF
cell B-reassign-return '2 1 2' B <<'EOF'
fn make() -> vector<vector<integer>> { z: vector<vector<integer>> = [[9]]; z = [[1],[2]]; z }
fn main() { d = make(); println("{len(d)} {d[0][0]} {d[1][0]}"); }
EOF

# ---- Cluster C: construction path ----
cell C-literal '2 1 2' C <<'EOF'
fn main() { z: vector<vector<integer>> = [[1],[2]]; println("{len(z)} {z[0][0]} {z[1][0]}"); }
EOF
cell C-literal-big '100 4950' C <<'EOF'
fn main() { z: vector<vector<integer>> = [];
  for i in 0..100 { z += [[i]]; } t=0; s=0; for j in 0..100 { t+=len(z[j]); s+=z[j][0]; } println("{t} {s}"); }
EOF
cell C-comprehension '10 285' C <<'EOF'
fn main() { z: vector<vector<integer>> = [ [i, i*2] | i in 0..10 ];
  t=0; s=0; for j in 0..10 { t+=len(z[j]); s+=z[j][0]+z[j][1]; } println("{t} {s}"); }
EOF
cell C-copy-assign '2 2' C <<'EOF'
fn main() { a: vector<vector<integer>> = [[1],[2]]; b = a;
  println("{len(b)} {b[1][0]}"); }
EOF

# ---- Cluster D: access shape ----
cell D-double-index '597600' D <<'EOF'
fn main() { adj: vector<vector<integer>> = [];
  for i in 0..200 { r: vector<integer>=[]; adj += [r]; }
  for j in 0..200 { for k in 0..3 { adj[j] += [j*10+k]; } }
  s=0; for j in 0..200 { for k in 0..3 { s += adj[j][k]; } } println("{s}"); }
EOF
cell D-nested-iter '90000' D <<'EOF'
fn main() { n=300; adj: vector<vector<integer>> = [];
  for i in 0..n { r: vector<integer>=[]; adj += [r]; } for j in 0..n { adj[j]+=[j]; adj[j]+=[j+1]; }
  s=0; for row in adj { for v in row { s+=v; } } println("{s}"); }
EOF
cell D-elem-assign '200' D <<'EOF'
fn main() { adj: vector<vector<integer>> = [];
  for i in 0..200 { adj += [[0]]; } for j in 0..200 { adj[j][0] = j; }
  c=0; for j in 0..200 { if adj[j][0]==j { c+=1; } } println("{c}"); }
EOF
cell D-pass-inner '44850' D <<'EOF'
fn sum(v: vector<integer>) -> integer { s=0; for x in v { s+=x; } s }
fn main() { adj: vector<vector<integer>> = [];
  for i in 0..300 { adj += [[i]]; } t=0; for j in 0..300 { t += sum(adj[j]); } println("{t}"); }
EOF

# ---- Cluster E: nesting depth / other collections ----
cell E-triple '50' E <<'EOF'
fn main() { a: vector<vector<vector<integer>>> = [];
  for i in 0..50 { r: vector<vector<integer>>=[]; a += [r]; }
  for j in 0..50 { inner: vector<integer>=[j,j+1]; a[j] += [inner]; }
  t=0; for j in 0..50 { if len(a[j])==1 && a[j][0][0]==j { t+=1; } } println("{t}"); }
EOF

# ---- Cluster F: scale (does append+read hold at large n) ----
cell F-large '5000 12497500' F <<'EOF'
fn main() { n=5000; adj: vector<vector<integer>> = [];
  for i in 0..n { r: vector<integer>=[]; adj += [r]; } for j in 0..n { adj[j] += [j]; }
  t=0; s=0; for j in 0..n { t+=len(adj[j]); s+=adj[j][0]; } println("{t} {s}"); }
EOF

# ---- Controls (must PASS everywhere) ----
cell ctrl-flat '499500' X <<'EOF'
fn main() { v: vector<integer>=[]; for i in 0..1000 { v+=[i]; } s=0; for x in v { s+=x; } println("{s}"); }
EOF
cell ctrl-vec-struct '124750' X <<'EOF'
struct P { x: integer not null, y: integer not null }
fn main() { v: vector<P>=[]; for i in 0..500 { v+=[P{x:i,y:0}]; } s=0; for p in v { s+=p.x; } println("{s}"); }
EOF

printf "%-8s %-18s %-14s | %-13s %-13s\n" CL CELL EXPECT INTERPRET NATIVE
for i in "${!NAMES[@]}"; do
  n="${NAMES[$i]}"; e="${EXPECT[$i]}"; cl="${CLUSTER[$i]}"; f="$D/$n.loft"
  oi=$(LOFT_TIMEOUT=60 timeout 90 "$LOFT" --interpret "$f" 2>/dev/null); ci=$?
  on=$(LOFT_TIMEOUT=120 timeout 150 "$LOFT" --native "$f" 2>/dev/null); cn=$?
  printf "%-8s %-18s %-14s | %-13s %-13s\n" "$cl" "$n" "$e" "$(classify $ci "$oi" "$e")" "$(classify $cn "$on" "$e")"
done
