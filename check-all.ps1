function Invoke-Step {
	param(
		[Parameter(Mandatory = $true)]
		[string]$Label,
		[Parameter(Mandatory = $true)]
		[scriptblock]$Action
	)

	Write-Host -ForegroundColor Yellow "Running $Label..."
	& $Action
	if ($LASTEXITCODE -ne 0) {
		throw "$Label failed with exit code $LASTEXITCODE"
	}
}

function Get-NonSubscriberTestFeatureArgs {
	$metadata = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
	$pkg = if ($metadata.packages.Count -eq 1) {
		$metadata.packages[0]
	} else {
		$manifestPath = (Resolve-Path (Join-Path (Get-Location) 'Cargo.toml')).Path
		$metadata.packages |
			Where-Object { $_.manifest_path -eq $manifestPath } |
			Select-Object -First 1
	}
	if (-not $pkg) {
		throw "Could not determine root package from cargo metadata"
	}

	$features = @($pkg.features.PSObject.Properties.Name | Where-Object { $_ -notin @("default", "tracing-subscriber") })
	if ($features.Count -gt 0) {
		return @("--features", ($features -join ","))
	}

	return @()
}

Invoke-Step -Label "format check" -Action {
	rustup run nightly -- cargo fmt --all
}

Invoke-Step -Label "clippy lint check" -Action {
	cargo clippy --all-features -- -D warnings
}

Invoke-Step -Label "build" -Action {
	cargo build --all-features --quiet
}

Invoke-Step -Label "examples" -Action {
	cargo check --all-features --examples --quiet
}

Invoke-Step -Label "tests" -Action {
	$featuresArg = Get-NonSubscriberTestFeatureArgs
	cargo test @featuresArg --quiet
}

Invoke-Step -Label "tracing-subscriber tests" -Action {
	cargo test --no-default-features --features tracing-subscriber --quiet
}
