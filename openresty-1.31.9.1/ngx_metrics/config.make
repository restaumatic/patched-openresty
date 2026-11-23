dir="../../ngx_metrics"
ar="${dir}/target/release/libngx_metrics.a"

cat << END                                            >> $NGX_MAKEFILE

$ar: ngx_metrics_cargo
	cd "$dir" && cargo build --release

.PHONY: ngx_metrics_cargo
ngx_metrics_cargo:

END
