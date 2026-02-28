package coremain

import "embed"

//go:embed www/install/*
var installFS embed.FS

//go:embed www/install/templates/config.yaml.template
var configTemplate embed.FS

//go:embed www/install/templates/rules/*
var rulesTemplate embed.FS

//go:embed www/install/templates/lists/*
var listsTemplate embed.FS
