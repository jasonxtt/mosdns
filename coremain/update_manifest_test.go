package coremain

import "testing"

func TestValidateReleaseUpdateManifest(t *testing.T) {
	status := UpdateStatus{
		LatestVersion: "v0.7.0",
		AssetName:     "mosdns-0.7.0-linux-amd64.tar.gz",
	}
	manifest := releaseUpdateManifest{
		Format:               updateManifestFormat,
		Channel:              "main",
		Version:              "v0.7.0",
		RequiredConfigSchema: 3,
		ConfigPackageID:      "main-config-schema-3",
		Artifacts: map[string]updateArtifact{
			status.AssetName: {SHA256: "cc520a52e059639b55d3388de23fc4ee6fe6f445f2879ac7ed88c4135ad604bc"},
		},
		Config: &updateConfigArtifact{
			URL:    "https://example.invalid/config_up.zip",
			SHA256: "cc520a52e059639b55d3388de23fc4ee6fe6f445f2879ac7ed88c4135ad604bc",
		},
	}
	if err := validateReleaseUpdateManifest(manifest, status); err != nil {
		t.Fatalf("valid manifest rejected: %v", err)
	}

	wrongVersion := manifest
	wrongVersion.Version = "v0.7.1"
	if err := validateReleaseUpdateManifest(wrongVersion, status); err == nil {
		t.Fatal("manifest for another version was accepted")
	}

	badHash := manifest
	badHash.Artifacts = map[string]updateArtifact{
		status.AssetName: {SHA256: "not-a-hash"},
	}
	if err := validateReleaseUpdateManifest(badHash, status); err == nil {
		t.Fatal("manifest with invalid artifact hash was accepted")
	}
}

func TestValidateReleaseUpdateManifestRequiresConfigMetadata(t *testing.T) {
	status := UpdateStatus{LatestVersion: "v0.7.0", AssetName: "mosdns.tar.gz"}
	manifest := releaseUpdateManifest{
		Format:               updateManifestFormat,
		Channel:              "main",
		Version:              status.LatestVersion,
		RequiredConfigSchema: 3,
		Artifacts: map[string]updateArtifact{
			status.AssetName: {SHA256: "cc520a52e059639b55d3388de23fc4ee6fe6f445f2879ac7ed88c4135ad604bc"},
		},
	}
	if err := validateReleaseUpdateManifest(manifest, status); err == nil {
		t.Fatal("manifest without config package id was accepted")
	}
}
