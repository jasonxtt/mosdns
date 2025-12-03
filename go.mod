module github.com/IrineSistiana/mosdns/v5

go 1.25.5

require (
	github.com/IrineSistiana/go-bytes-pool v0.0.0-20230918115058-c72bd9761c57
	github.com/go-chi/chi/v5 v5.2.3
	github.com/go-viper/mapstructure/v2 v2.4.0
	github.com/google/nftables v0.3.0
	github.com/google/uuid v1.6.0
	github.com/kardianos/service v1.2.4
	github.com/klauspost/compress v1.18.2
	github.com/miekg/dns v1.1.68
	github.com/nadoo/ipset v0.5.0
	github.com/prometheus/client_golang v1.23.2
	github.com/quic-go/quic-go v0.57.1
	github.com/robfig/cron/v3 v3.0.1
	github.com/sagernet/sing v0.6.7
	github.com/spf13/cobra v1.10.1
	github.com/spf13/viper v1.21.0
	github.com/stretchr/testify v1.11.1
	github.com/vishvananda/netlink v1.3.1
	go.uber.org/zap v1.27.1
	go4.org/netipx v0.0.0-20231129151722-fdeea329fbba
	golang.org/x/exp v0.0.0-20241210194714-1829a127f884
	golang.org/x/net v0.47.0
	golang.org/x/sync v0.18.0
	golang.org/x/sys v0.38.0
	golang.org/x/time v0.14.0
	google.golang.org/protobuf v1.36.9
	gopkg.in/yaml.v3 v3.0.1
)

replace github.com/nadoo/ipset v0.5.0 => github.com/IrineSistiana/ipset v0.5.1-0.20220703061533-6e0fc3b04c0a

require (
	github.com/beorn7/perks v1.0.1 // indirect
	github.com/cespare/xxhash/v2 v2.3.0 // indirect
	github.com/davecgh/go-spew v1.1.2-0.20180830191138-d8f796af33cc // indirect
	github.com/fsnotify/fsnotify v1.9.0 // indirect
	github.com/google/go-cmp v0.7.0 // indirect
	github.com/inconshreveable/mousetrap v1.1.0 // indirect
	github.com/mdlayher/netlink v1.7.3-0.20250113171957-fbb4dce95f42 // indirect
	github.com/mdlayher/socket v0.5.1 // indirect
	github.com/munnerz/goautoneg v0.0.0-20191010083416-a7dc8b61c822 // indirect
	github.com/pelletier/go-toml/v2 v2.2.4 // indirect
	github.com/pmezard/go-difflib v1.0.1-0.20181226105442-5d4384ee4fb2 // indirect
	github.com/prometheus/client_model v0.6.2 // indirect
	github.com/prometheus/common v0.66.1 // indirect
	github.com/prometheus/procfs v0.16.1 // indirect
	github.com/quic-go/qpack v0.6.0 // indirect
	github.com/sagikazarmark/locafero v0.11.0 // indirect
	github.com/sourcegraph/conc v0.3.1-0.20240121214520-5f936abd7ae8 // indirect
	github.com/spf13/afero v1.15.0 // indirect
	github.com/spf13/cast v1.10.0 // indirect
	github.com/spf13/pflag v1.0.10 // indirect
	github.com/subosito/gotenv v1.6.0 // indirect
	github.com/vishvananda/netns v0.0.5 // indirect
	go.uber.org/multierr v1.11.0 // indirect
	go.yaml.in/yaml/v2 v2.4.2 // indirect
	go.yaml.in/yaml/v3 v3.0.4 // indirect
	golang.org/x/crypto v0.44.0 // indirect
	golang.org/x/mod v0.29.0 // indirect
	golang.org/x/text v0.31.0 // indirect
	golang.org/x/tools v0.38.0 // indirect
)
