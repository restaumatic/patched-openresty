dir="../../ngx_metrics"
ar="${dir}/target/release/libngx_metrics.a"

cat << END                                            >> $NGX_MAKEFILE

$ar: $(find ${dir}/src ${dir}/Cargo.toml -type f | while read f; do echo -n "$f "; done)
	cd "$dir" && cargo build --release

END
