<#
.SYNOPSIS
    E2E Test Executor for service modules.

.DESCRIPTION
    Parses a YAML spec (Given/When/Then structure), executes the complete E2E lifecycle (given, startup,
    health-check, scenarios, cleanup), and outputs structured JSON evidence.
    PowerShell 5.1 compatible - no external PS modules required.
    Uses Python (PyYAML) for YAML-to-JSON conversion.

.PARAMETER YamlSpec
    Path to the YAML specification file defining the E2E test suite.

.PARAMETER Module
    Module name used for evidence file naming and metadata.

.PARAMETER EvidenceDir
    Output directory for the JSON evidence file.

.EXAMPLE
    .\run-e2e.ps1 -YamlSpec .\e2e-runtime.yaml -Module runtime -EvidenceDir .\evidence
#>

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$YamlSpec,

    # Optional: Module name used for evidence file naming (e.g., "mirror" → mirror-e2e-result.json).
    # If not provided, automatically inferred from the YAML filename by stripping common suffixes:
    #   mirror-e2e-test.yaml → mirror
    #   runtime-test.yaml    → runtime
    #   connectors.yaml      → connectors
    [Parameter(Mandatory = $false)]
    [string]$Module = "",

    [Parameter(Mandatory = $true)]
    [string]$EvidenceDir,

    # Optional: directory to write service process stdout/stderr log files.
    # When set, stdout and stderr of every started service process are
    # redirected to timestamped files in this directory, and the paths are
    # recorded in the evidence JSON under "service_log". This allows the
    # consuming Agent to read service logs for post-mortem analysis on
    # FAILED test runs.
    [Parameter(Mandatory = $false)]
    [string]$ServiceLogDir = "",

    # Optional: global auto-increment counter for ${seq_id} replacement in all commands.
    # The model provides the initial value; the script replaces ${seq_id} with the current
    # counter in every command (build_command, startup_command, when.commands, cleanup.commands)
    # and increments the counter after each replacement.
    [Parameter(Mandatory = $false)]
    [int]$SeqId = 0
)

# Initialize global seq_id counter from CLI parameter
$GlobalSeqId = $SeqId

# ============================================================================
# SCRIPT VERSION AND CONSTANTS
# ============================================================================
$SCRIPT_VERSION = "1.0.0"
$UTC_NOW = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ", [System.Globalization.CultureInfo]::InvariantCulture)

# ============================================================================
# AUTO-INFER MODULE NAME (when not provided explicitly)
# ============================================================================
# Strips common suffixes from the YAML basename (case-insensitive):
#   mirror-e2e-test.yaml  → mirror
#   runtime-test.yaml     → runtime
#   connectors.yaml       → connectors
if ([string]::IsNullOrWhiteSpace($Module)) {
    $yamlBaseName = [System.IO.Path]::GetFileNameWithoutExtension($YamlSpec)
    # Remove common suffixes in order of longest-match-first
    $inferred = $yamlBaseName -replace '-e2e-test$', ''
    if ($inferred -eq $yamlBaseName) {
        $inferred = $yamlBaseName -replace '[-_]e2e[-_]test$', ''
    }
    if ($inferred -eq $yamlBaseName) {
        $inferred = $yamlBaseName -replace '-e2e$', ''
    }
    if ($inferred -eq $yamlBaseName) {
        $inferred = $yamlBaseName -replace '[-_]test$', ''
    }
    # Fallback: if nothing matched but the original was non-empty, use it as-is
    if ([string]::IsNullOrWhiteSpace($inferred)) {
        $inferred = $yamlBaseName
    }
    if ([string]::IsNullOrWhiteSpace($inferred)) {
        Write-Error "Cannot infer module name from YAML file: $YamlSpec. Please pass -Module explicitly."
        exit 1
    }
    $Module = $inferred
    Write-Host "Module name not provided; inferred from YAML filename: '$Module'"
}

# ============================================================================
# UTF-8 OUTPUT SETUP (PowerShell 5.1)
# ============================================================================
$OutputEncoding = [System.Text.Encoding]::UTF8
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

# ============================================================================
# HELPER: Convert YAML to JSON via Python
# ============================================================================
function ConvertYamlToJson {
    param(
        [Parameter(Mandatory = $true)]
        [string]$YamlPath
    )

    if (-not (Test-Path -LiteralPath $YamlPath)) {
        Write-Error "YAML file not found: $YamlPath"
        return $null
    }

    $yamlContent = Get-Content -LiteralPath $YamlPath -Raw -Encoding UTF8

    # Use Python with PyYAML to convert YAML to JSON
    # The python script reads YAML and dumps as JSON to stdout
    $pythonScript = @"
import yaml
import json
import sys

with open(sys.argv[1], 'r', encoding='utf-8') as f:
    data = yaml.safe_load(f)

json.dump(data, sys.stdout, ensure_ascii=False, indent=2)
"@

    $tempPyFile = Join-Path $env:TEMP "e2e_yaml_convert_$($Module)_$(Get-Random).py"
    try {
        Set-Content -LiteralPath $tempPyFile -Value $pythonScript -Encoding UTF8

        $pythonResult = & python $tempPyFile $YamlSpec 2>&1

        if ($LASTEXITCODE -ne 0) {
            Write-Error "Python YAML conversion failed: $pythonResult"
            return $null
        }

# PowerShell 5.1 ConvertFrom-Json returns a PSCustomObject for single objects
        # and an array for arrays. Default depth in PS 5.1 is 20 which is sufficient
        # for our YAML structure. PS 7+ supports -Depth parameter, but PS 5.1 does not.
        # Use -Depth only if the PowerShell version supports it (7.0+).
        $psMajorVersion = $PSVersionTable.PSVersion.Major
        if ($psMajorVersion -ge 7) {
            $jsonObject = $pythonResult | ConvertFrom-Json -Depth 20
        }
        else {
            $jsonObject = $pythonResult | ConvertFrom-Json
        }
        return $jsonObject
    }
    finally {
        if (Test-Path -LiteralPath $tempPyFile) {
            Remove-Item -LiteralPath $tempPyFile -Force -ErrorAction SilentlyContinue
        }
    }
}

# ============================================================================
# HELPER: Safe HTTP request wrapper
# ============================================================================
function InvokeSafeWebRequest {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Url,

        [Parameter(Mandatory = $false)]
        [string]$Method = "GET",

        [Parameter(Mandatory = $false)]
        [hashtable]$Headers = @{},

        [Parameter(Mandatory = $false)]
        [string]$Body = "",

        [Parameter(Mandatory = $false)]
        [int]$TimeoutSeconds = 30
    )

    $result = @{
        StatusCode   = 0
        Body         = ""
        ErrorMessage = ""
        Success      = $false
    }

    try {
        $invokeParams = @{
            Uri               = $Url
            Method            = $Method
            UseBasicParsing   = $true
            TimeoutSec        = $TimeoutSeconds
            ErrorAction       = "Stop"
        }

        # Add headers if provided
        if ($Headers.Count -gt 0) {
            $invokeParams["Headers"] = $Headers
        }

        # Add body if provided (for POST, PUT, DELETE)
        if (($Method -eq "POST" -or $Method -eq "PUT" -or $Method -eq "DELETE") -and $Body -ne "") {
            $invokeParams["Body"] = $Body
            # Default content type for JSON payloads
            if (-not $Headers.ContainsKey("Content-Type")) {
                $invokeParams["ContentType"] = "application/json"
            }
        }

        $response = Invoke-WebRequest @invokeParams

        $result.StatusCode = $response.StatusCode
        $result.Success = $true

        # PowerShell 5.1 quirk: response.Content is a string but may need explicit
        # handling for JSON responses. Extract raw content string.
        if ($response.Content -is [string]) {
            $result.Body = $response.Content
        }
        elseif ($response.Content -is [byte[]]) {
            # Binary content - decode as UTF-8
            $result.Body = [System.Text.Encoding]::UTF8.GetString($response.Content)
        }
        else {
            $result.Body = $response.Content.ToString()
        }
    }
catch [System.Net.WebException] {
        # Handle HTTP errors (4xx, 5xx) - PowerShell 5.1 throws even for 4xx
        $ex = $_.Exception
        if ($ex.Response -ne $null) {
            try {
                $httpResponse = $ex.Response
                $result.StatusCode = [int]$httpResponse.StatusCode
                # Read error response body
                $stream = $httpResponse.GetResponseStream()
                # Reset stream position - PS 5.1 Invoke-WebRequest consumes stream internally,
                # leaving position at end. Must reset to 0 before reading error response body.
                if ($stream.CanSeek) {
                    $stream.Seek(0, [System.IO.SeekOrigin]::Begin)
                }
                $reader = New-Object System.IO.StreamReader($stream, [System.Text.Encoding]::UTF8)
                $result.Body = $reader.ReadToEnd()
                $reader.Close()
                $stream.Close()
                # We got a status code back, so consider this "success" in terms of
                # receiving a response (even if it's an error status)
                $result.Success = $true
            }
            catch {
                $result.ErrorMessage = "HTTP error but could not read response: $($_.Exception.Message)"
                $result.StatusCode = 0
            }
        }
        else {
            $result.ErrorMessage = "WebException without response: $ex.Message"
        }
    }
catch {
        # Dynamic type check: PS 7+ throws HttpResponseException, PS 5.1 throws WebException
        # We must check dynamically because [HttpResponseException] type doesn't exist in PS 5.1
        # and would cause a parse-time type resolution error if used in a typed catch clause.
        $isHttpResponseException = ($PSVersionTable.PSVersion.Major -ge 7 -and $_.Exception.GetType().FullName -eq "Microsoft.PowerShell.Commands.HttpResponseException")

        if ($isHttpResponseException) {
            # Handle HTTP errors (4xx, 5xx) - PowerShell 7+ throws HttpResponseException
            $ex = $_.Exception
            try {
                $result.StatusCode = [int]$ex.Response.StatusCode
                # Read error response body from PS 7+ HttpResponseException
                $stream = $ex.Response.RawContentStream
                $stream.Position = 0
                $reader = New-Object System.IO.StreamReader($stream, [System.Text.Encoding]::UTF8)
                $result.Body = $reader.ReadToEnd()
                $reader.Close()
                $stream.Close()
                # We got a status code back, so consider this "success" in terms of
                # receiving a response (even if it's an error status)
                $result.Success = $true
            }
            catch {
                # Fallback: try reading from RawContent (string property in PS 7+)
                try {
                    $result.StatusCode = [int]$ex.Response.StatusCode
                    if ($ex.Response.RawContent -ne $null) {
                        $result.Body = $ex.Response.RawContent.ToString()
                    }
                    $result.Success = $true
                }
                catch {
                    $result.ErrorMessage = "HTTP error but could not read response (PS 7+): $($_.Exception.Message)"
                    $result.StatusCode = 0
                }
            }
        }
        else {
            # Other errors (connection refused, timeout, DNS failure, etc.)
            $result.ErrorMessage = $_.Exception.Message
        }
    }

    return $result
}

# ============================================================================
# HELPER: Wait for service readiness (HTTP health probe)
# ============================================================================
# Polls the given readiness URL until the service responds with any HTTP status
# code (indicating it is listening). Connection errors (refused, timeout, DNS)
# are treated as "not ready yet".
#
# Parameters:
#   - ProbeUrl:        The URL to poll (typically ${base_url} or a health endpoint)
#   - TimeoutSeconds:  Maximum wait time before declaring startup failure
#   - IntervalSeconds: Seconds between poll attempts
#
# Returns:
#   $true  if the service is responding within the timeout
#   $false if the service did not respond within the timeout
function WaitForServiceReadiness {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ProbeUrl,

        [Parameter(Mandatory = $false)]
        [int]$TimeoutSeconds = 60,

        [Parameter(Mandatory = $false)]
        [int]$IntervalSeconds = 2
    )

    Write-Host "Readiness probe: polling $ProbeUrl (timeout=${TimeoutSeconds}s, interval=${IntervalSeconds}s)"

    $ready = $false
    $elapsed = 0
    while ($elapsed -lt $TimeoutSeconds) {
        try {
            $webResult = Invoke-WebRequest -Uri $ProbeUrl `
                -Method GET `
                -UseBasicParsing `
                -TimeoutSec 5 `
                -ErrorAction Stop
            # Any successful HTTP response (2xx/3xx): service is ready
            Write-Host "  Service ready: HTTP $($webResult.StatusCode) after ${elapsed}s"
            $ready = $true
            break
        }
        catch {
            # Connection-level errors: service not listening yet. Expected during startup.
            # HTTP error responses (4xx/5xx) also mean service is listening → ready
            if ($_.Exception.GetType().Name -match "WebException|HttpRequestException") {
                $ex = $_.Exception
                if ($ex.Response -ne $null) {
                    # We got an HTTP response (even 4xx/5xx): service is listening
                    $statusCode = [int]$ex.Response.StatusCode
                    Write-Host "  Service ready (HTTP $statusCode) after ${elapsed}s"
                    $ready = $true
                    break
                }
                # No response = connection not yet available, keep polling silently
            }
        }

        Start-Sleep -Seconds $IntervalSeconds
        $elapsed += $IntervalSeconds
    }

    if (-not $ready) {
        Write-Host "  Readiness probe TIMEOUT after ${TimeoutSeconds}s — service not responding at $ProbeUrl"
    }

    return $ready
}

# ============================================================================
# HELPER: Check dependencies (TCP probes from YAML spec)
# ============================================================================
function CheckDependencies {
    param(
        [Parameter(Mandatory = $true)]
        [object]$SpecData
    )

    $deps = $SpecData.dependencies
    if ($null -eq $deps) { return @{ Available = $true; Checked = @(); FailedName = ""; FailedHost = ""; FailedPort = 0 } }

    # Normalize to array
    if ($deps -isnot [array]) { $deps = @($deps) }

    $checkedNames = @()
    foreach ($dep in $deps) {
        $name = if ($dep.name -ne $null) { $dep.name.ToString() } else { "unnamed" }
        $host_ = if ($dep.host -ne $null) { $dep.host.ToString() } else { "localhost" }
        $port = if ($dep.port -ne $null) { [int]$dep.port } else { 0 }

        $checkedNames += $name

        Write-Host "  Checking dependency '$name' at ${host_}:${port}..."

        if ($port -le 0) {
            Write-Host "  Dependency '$name' has no valid port, skipping"
            continue
        }

        $tcpClient = New-Object System.Net.Sockets.TcpClient
        try {
            $connectResult = $tcpClient.BeginConnect($host_, $port, $null, $null)
            $waitResult = $connectResult.AsyncWaitHandle.WaitOne(5000, $false)
            if (-not $waitResult -or -not $tcpClient.Connected) {
                Write-Host "  Dependency '$name' unavailable at ${host_}:${port}"
                $tcpClient.Close()
                return @{ Available = $false; Checked = $checkedNames; FailedName = $name; FailedHost = $host_; FailedPort = $port }
            }
            $tcpClient.EndConnect($connectResult)
            $tcpClient.Close()
            Write-Host "  Dependency '$name' available"
        } catch {
            Write-Host "  Dependency '$name' unavailable at ${host_}:${port}: $($_.Exception.Message)"
            $tcpClient.Close()
            return @{ Available = $false; Checked = $checkedNames; FailedName = $name; FailedHost = $host_; FailedPort = $port }
        }
    }
    return @{ Available = $true; Checked = $checkedNames; FailedName = ""; FailedHost = ""; FailedPort = 0 }
}

# ============================================================================
# HELPER: Validate status code (supports single int or array)
# ============================================================================
function ValidateStatusCode {
    param(
        [Parameter(Mandatory = $true)]
        [int]$ReceivedCode,

        [Parameter(Mandatory = $true)]
        $ExpectedStatus
    )

    # ExpectedStatus can be a single integer or an array of integers
    if ($ExpectedStatus -is [array]) {
        foreach ($expected in $ExpectedStatus) {
            if ($ReceivedCode -eq [int]$expected) {
                return $true
            }
        }
        return $false
    }
    else {
        return ($ReceivedCode -eq [int]$ExpectedStatus)
    }
}

# ============================================================================
# HELPER: Validate body (strict JSON map matching)
# ============================================================================
# The 'body' assertion in YAML provides an expected map (key-value pairs).
# Every key in the expected map must be present in the actual response body,
# and the value must match exactly (deep match is not performed - top-level
# keys only; values must be equal as strings).
# Extra keys in the actual response are ignored.
function ValidateBodyStrict {
    param(
        [Parameter(Mandatory = $false)]
        [AllowEmptyString()]
        [AllowNull()]
        [string]$ResponseBody = "",

        # Expected map from YAML (already parsed by PyYAML into a PSCustomObject
        # when passed through ConvertYamlToJson; may also be a hashtable.)
        [Parameter(Mandatory = $true)]
        [AllowNull()]
        $ExpectedBody
    )

    if ($ExpectedBody -eq $null) { return $true }

    # Normalize expected into a hashtable
    $expectedMap = @{}
    if ($ExpectedBody -is [hashtable]) {
        $expectedMap = $ExpectedBody
    }
    elseif ($ExpectedBody -is [System.Collections.IDictionary]) {
        foreach ($key in $ExpectedBody.Keys) { $expectedMap[$key.ToString()] = $ExpectedBody[$key.ToString()] }
    }
    elseif ($ExpectedBody -is [PSCustomObject]) {
        foreach ($prop in $ExpectedBody.PSObject.Properties) {
            $expectedMap[$prop.Name] = $prop.Value
        }
    }
    else {
        # Cannot parse expected body as a map
        Write-Host "  body assertion: expected value is not a map; skipping"
        return $false
    }

    if ($expectedMap.Count -eq 0) { return $true }

    # Parse actual response body as JSON
    if ([string]::IsNullOrEmpty($ResponseBody)) {
        return $false
    }

    $actualObj = $null
    try {
        $psMajorVersion = $PSVersionTable.PSVersion.Major
        if ($psMajorVersion -ge 7) {
            $actualObj = $ResponseBody | ConvertFrom-Json -Depth 10 -ErrorAction Stop
        }
        else {
            $actualObj = $ResponseBody | ConvertFrom-Json -ErrorAction Stop
        }
    }
    catch {
        Write-Host "  body assertion: failed to parse response as JSON: $($_.Exception.Message)"
        return $false
    }

    # Compare each expected key-value pair top-level
    $actualAsHashtable = @{}
    if ($actualObj -is [PSCustomObject]) {
        foreach ($prop in $actualObj.PSObject.Properties) {
            $actualAsHashtable[$prop.Name] = $prop.Value
        }
    }
    elseif ($actualObj -is [hashtable]) {
        $actualAsHashtable = $actualObj
    }
    else {
        # Actual is not a map (array, scalar, etc.)
        Write-Host "  body assertion: response is not a JSON object"
        return $false
    }

    foreach ($entry in $expectedMap.GetEnumerator()) {
        $key = $entry.Key
        $expectedValue = $entry.Value

        if (-not $actualAsHashtable.ContainsKey($key)) {
            Write-Host "  body assertion: missing key '$key'"
            return $false
        }

        $actualValue = $actualAsHashtable[$key]

        # Compare as strings (YAML scalars are strings; JSON scalars may be
        # int/float/bool. Normalize both sides to .ToString() for equality.)
        $expectedStr = if ($expectedValue -eq $null) { "" } else { $expectedValue.ToString() }
        $actualStr = if ($actualValue -eq $null) { "" } else { $actualValue.ToString() }

        if ($expectedStr -ne $actualStr) {
            Write-Host "  body assertion: key '$key' mismatch - expected='$expectedStr', actual='$actualStr'"
            return $false
        }
    }

    return $true
}

# ============================================================================
# HELPER: Validate body_contains (case-insensitive substring match)
# ============================================================================
function ValidateBodyContains {
    param(
        [Parameter(Mandatory = $false)]
        [AllowEmptyString()]
        [AllowNull()]
        [string]$ResponseBody = "",

        [Parameter(Mandatory = $true)]
        $ExpectedSubstrings
    )

    # If response body is null or empty, body_contains cannot match any substring
    if ([string]::IsNullOrEmpty($ResponseBody)) {
        # If no expected substrings, it's trivially true
        if ($ExpectedSubstrings -eq $null -or ($ExpectedSubstrings -is [array] -and $ExpectedSubstrings.Count -eq 0)) {
            return $true
        }
        return $false
    }

    # ExpectedSubstrings can be a single string or an array of strings
    # Use .Contains() for reliable case-insensitive substring match (avoids -like wildcard issues)
    $responseLower = $ResponseBody.ToLower()
    if ($ExpectedSubstrings -is [array]) {
        foreach ($substr in $ExpectedSubstrings) {
            $substrLower = $substr.ToString().ToLower()
            if (-not $responseLower.Contains($substrLower)) {
                return $false
            }
        }
        return $true
    }
    else {
        $expectedLower = $ExpectedSubstrings.ToString().ToLower()
        return $responseLower.Contains($expectedLower)
    }
}

# ============================================================================
# HELPER: Validate body_not_contains (case-insensitive, ensures substrings
# are NOT present in the response body)
# ============================================================================
function ValidateBodyNotContains {
    param(
        [Parameter(Mandatory = $false)]
        [AllowEmptyString()]
        [AllowNull()]
        [string]$ResponseBody = "",

        [Parameter(Mandatory = $true)]
        $ExpectedAbsentSubstrings
    )

    # If no expected absent substrings, trivially true
    if ($ExpectedAbsentSubstrings -eq $null -or ($ExpectedAbsentSubstrings -is [array] -and $ExpectedAbsentSubstrings.Count -eq 0)) {
        return $true
    }

    # If response body is null or empty, absent substrings trivially not found (pass)
    if ([string]::IsNullOrEmpty($ResponseBody)) {
        return $true
    }

    $responseLower = $ResponseBody.ToLower()
    if ($ExpectedAbsentSubstrings -is [array]) {
        foreach ($substr in $ExpectedAbsentSubstrings) {
            $substrLower = $substr.ToString().ToLower()
            if ($responseLower.Contains($substrLower)) {
                return $false
            }
        }
        return $true
    }
    else {
        $absentLower = $ExpectedAbsentSubstrings.ToString().ToLower()
        return (-not $responseLower.Contains($absentLower))
    }
}

# ============================================================================
# HELPER: Extract value from JSON response body using dot-separated path
# ============================================================================
function ExtractJsonValue {
    param(
        [Parameter(Mandatory = $false)]
        [AllowEmptyString()]
        [AllowNull()]
        [string]$ResponseBody = "",

        [Parameter(Mandatory = $true)]
        [string]$JsonPath
    )

    if ([string]::IsNullOrEmpty($ResponseBody)) {
        return $null
    }

    try {
        # Convert JSON to PowerShell object
        $psMajorVersion = $PSVersionTable.PSVersion.Major
        if ($psMajorVersion -ge 7) {
            $jsonObj = $ResponseBody | ConvertFrom-Json -Depth 20
        }
        else {
            $jsonObj = $ResponseBody | ConvertFrom-Json
        }

        # Normalize jq-style path: strip leading "$." or "$" prefix so that
        # both "$.name" and "name" resolve to the same property.
        # Also handles bare "$" (root path).
        $normalizedPath = $JsonPath
        if ($normalizedPath.StartsWith("$.")) {
            $normalizedPath = $normalizedPath.Substring(2)
        }
        elseif ($normalizedPath.StartsWith("$")) {
            $normalizedPath = $normalizedPath.Substring(1)
        }

        # Bare "$" or empty after strip → return the entire JSON object
        if ([string]::IsNullOrEmpty($normalizedPath)) {
            return $jsonObj.ToString()
        }

        # Navigate dot-separated path (e.g., "name", "tasks[0].id")
        $pathParts = $normalizedPath.Split(".")
        $current = $jsonObj

        foreach ($part in $pathParts) {
            if ($current -eq $null) {
                return $null
            }

            # Handle array index notation like "tasks[0]"
            if ($part -match '^(\w+)\[(\d+)\]$') {
                $arrayProp = $Matches[1]
                $arrayIndex = [int]$Matches[2]
                if ($current -is [array]) {
                    # If current is already an array, index into it then get property
                    $current = $current[$arrayIndex]
                    if ($arrayProp -ne $null -and $arrayProp -ne "") {
                        $current = $current.$arrayProp
                    }
                }
                else {
                    $current = $current.$arrayProp
                    if ($current -is [array]) {
                        $current = $current[$arrayIndex]
                    }
                }
            }
            elseif ($current -is [array]) {
                # If current is an array and path is numeric index
                if ($part -match '^\d+$') {
                    $current = $current[[int]$part]
                }
                else {
                    # Access property on first element of array
                    $current = $current[0].$part
                }
            }
            else {
                $current = $current.$part
            }
        }

        if ($current -eq $null) {
            return $null
        }

        return $current.ToString()
    }
    catch {
        return $null
    }
}

# ============================================================================
# HELPER: Convert PSCustomObject to nested hashtable for JSON serialization
# ============================================================================
function ConvertToNestedHashtable {
    param(
        [Parameter(Mandatory = $true)]
        [object]$InputObject
    )

    if ($InputObject -is [PSCustomObject]) {
        $hashtable = @{}
        foreach ($property in $InputObject.PSObject.Properties) {
            $hashtable[$property.Name] = ConvertToNestedHashtable -InputObject $property.Value
        }
        return $hashtable
    }
    elseif ($InputObject -is [array]) {
        $resultArray = @()
        foreach ($item in $InputObject) {
            $resultArray += ConvertToNestedHashtable -InputObject $item
        }
        return $resultArray
    }
    else {
        # Primitive value (string, int, bool, null) - return as-is
        return $InputObject
    }
}

# ============================================================================
# HELPER: Write structured JSON evidence
# ============================================================================
function WriteEvidenceJson {
    param(
        [Parameter(Mandatory = $true)]
        [hashtable]$Evidence,

        [Parameter(Mandatory = $true)]
        [string]$OutputPath
    )

# Ensure output directory exists
    # Use [System.IO.Path] for PS 5.1 compatibility (Split-Path -LiteralPath -Parent has ambiguity)
    $outputDir = [System.IO.Path]::GetDirectoryName($OutputPath)
    if (-not (Test-Path -LiteralPath $outputDir)) {
        New-Item -ItemType Directory -LiteralPath $outputDir -Force | Out-Null
    }

    # Convert hashtable to JSON with proper depth
    $jsonString = ConvertTo-Json -InputObject $Evidence -Depth 20 -Compress:$false

    # Write with UTF-8 encoding (no BOM for cross-platform compatibility)
    $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($OutputPath, $jsonString, $utf8NoBom)

    Write-Host "Evidence written to: $OutputPath"
}

# ============================================================================
# HELPER: Stop all service processes started during given phase
# ============================================================================
function Stop-AllServiceProcesses {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Phase  # "cleanup" or "teardown"
    )

    if ($ServiceProcessIds.Count -eq 0) { return }

    foreach ($pid in $ServiceProcessIds) {
        Write-Host "  Stopping service process (PID: $pid) [$Phase]..."
        try {
            Stop-Process -Id $pid -Force -ErrorAction Stop
            Start-Sleep -Seconds 2
            $stillRunning = Get-Process -Id $pid -ErrorAction SilentlyContinue
            if ($stillRunning -ne $null) {
                Write-Host "  Process $pid still running, retrying kill..."
                Stop-Process -Id $pid -Force -ErrorAction SilentlyContinue
                Start-Sleep -Seconds 3
            }
            Write-Host "  Process $pid stopped"
        }
        catch {
            # Fallback to taskkill
            try {
                & taskkill /F /PID $pid 2>&1 | Out-Null
                Start-Sleep -Seconds 1
                Write-Host "  Process $pid killed via taskkill"
            }
            catch {
                Write-Host "  Could not stop process ${pid}: $($_.Exception.Message)"
            }
        }
    }
}

# ============================================================================
# HELPER: Build Skipped evidence (dependencies unavailable)
# ============================================================================
function BuildSkippedEvidence {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Module,

        [Parameter(Mandatory = $true)]
        [string]$YamlFile,

        [Parameter(Mandatory = $false)]
        [string]$SkipReason = "Dependencies unavailable",

        [Parameter(Mandatory = $false)]
        [array]$DependenciesChecked = @()
    )

    return @{
        metadata = @{
            module        = $Module
            yaml_file     = $YamlFile
            executed_at   = $UTC_NOW
            environment   = "PowerShell"
            script_version = $SCRIPT_VERSION
        }
prerequisite_check = @{
            dependencies_available = $false
            dependencies_checked   = $DependenciesChecked
            build_success        = $null
            startup_success      = $false
            startup_duration_seconds = 0
            log_expectations_met = $null
            priority             = "LOW"
        }
        tests    = @()
        cleanup  = @{
            resources_deleted = @()
            process_stopped   = $false
        }
        service_log = @{
            service_logs = @{}
        }
        summary  = @{
            total_tests    = 0
            passed         = 0
            failed         = 0
            skipped        = 0
            high_priority_failures = 0
            low_priority_failures  = 0
            blocking       = $false
            overall_result = "SKIPPED"
            skipped_reason = $SkipReason
        }
    }
}

# ============================================================================
# MAIN EXECUTION
# ============================================================================
Write-Host "========================================"
Write-Host "E2E Test Executor v$SCRIPT_VERSION"
Write-Host "Module: $Module"
Write-Host "YAML Spec: $YamlSpec"
Write-Host "Evidence Dir: $EvidenceDir"
Write-Host "========================================"

# Initialize mutable state (used throughout and in finally block)
$ServiceProcesses = @()
$ServiceProcessIds = @()
# Per-PID service log file paths (stdout + stderr combined into one file per process).
# Populated only when $ServiceLogDir is set (via -ServiceLogDir parameter).
# Structure: hashtable { PID = "absolute/path/to/service-PID.log" }
$ServiceLogFiles = @{}
$PrereqCheck = @{
    dependencies_available = $true
    dependencies_checked   = @()
    build_success        = $null
    startup_success      = $false
    startup_duration_seconds = 0
    log_expectations_met = $null
    priority             = "LOW"
}
$TestResults = @()
$CleanupResult = @{
    resources_deleted = @()
    process_stopped   = $false
}
$OverallResult = "FAIL"
$SkippedReason = ""
$ExitCode = 1

# Resolve ServiceLogDir to absolute path (if provided) and ensure it exists
# The model (Agent) passes this path so the service process stdout/stderr
# can be captured and read later for post-mortem analysis.
if ($ServiceLogDir -ne "") {
    if (-not [System.IO.Path]::IsPathRooted($ServiceLogDir)) {
        $ServiceLogDir = Join-Path $PWD $ServiceLogDir
    }
    if (-not (Test-Path -LiteralPath $ServiceLogDir)) {
        # Note: PowerShell 5.1 New-Item does not support -LiteralPath for directory creation;
        # use -Path instead. We don't expect wildcard characters in log paths.
        New-Item -ItemType Directory -Path $ServiceLogDir -Force | Out-Null
    }
    Write-Host "Service log directory: $ServiceLogDir"
}

# Resolve evidence output path
$EvidenceOutputPath = Join-Path $EvidenceDir "$Module-e2e-result.json"

# Resolve YamlSpec to absolute path
$YamlSpecAbsolute = Resolve-Path -LiteralPath $YamlSpec -ErrorAction SilentlyContinue
if ($YamlSpecAbsolute -eq $null) {
    Write-Error "YAML spec file not found: $YamlSpec"
    $skipEvidence = BuildSkippedEvidence -Module $Module -YamlFile $YamlSpec -SkipReason "YAML spec file not found: $YamlSpec"
    WriteEvidenceJson -Evidence $skipEvidence -OutputPath $EvidenceOutputPath
    exit 0
}
$YamlSpecAbsolute = $YamlSpecAbsolute.ToString()

# Parse YAML spec
Write-Host "Parsing YAML spec..."
$Spec = ConvertYamlToJson -YamlPath $YamlSpecAbsolute

if ($Spec -eq $null) {
    Write-Error "Failed to parse YAML spec"
    $skipEvidence = BuildSkippedEvidence -Module $Module -YamlFile $YamlSpecAbsolute -SkipReason "Failed to parse YAML spec"
    WriteEvidenceJson -Evidence $skipEvidence -OutputPath $EvidenceOutputPath
    exit 0
}

Write-Host "YAML spec parsed successfully: $($Spec.name)"

# ============================================================================
# Read module from YAML (overrides command-line -Module if present)
# ============================================================================
if ($Spec.module -ne $null -and $Spec.module -ne "") {
    $YamlModule = $Spec.module.ToString()
    if ($YamlModule -ne $Module) {
        Write-Host "Module from YAML: '$YamlModule' (overriding '$Module')"
        $Module = $YamlModule
    }
}

# Re-resolve evidence output path (now that $Module has its final value from YAML)
$EvidenceOutputPath = Join-Path $EvidenceDir "$Module-e2e-result.json"

# ============================================================================
# Read base_url from YAML spec (for URL substitution throughout)
# ============================================================================
$GlobalBaseUrl = ""
if ($Spec.base_url -ne $null -and $Spec.base_url -ne "") {
    $GlobalBaseUrl = $Spec.base_url.ToString()
}
if ($GlobalBaseUrl -ne "") {
    Write-Host "Base URL: $GlobalBaseUrl"
}

Write-Host "seq_id (auto-increment counter, initial): $GlobalSeqId"

# ============================================================================
# PHASE 0.5: SSL Certificate Verification Bypass (optional)
# ============================================================================
# When skip_ssl_verify: true is set in YAML, globally disable SSL certificate
# validation for all HTTP requests. Useful for testing self-signed certificate
# deployments (HTTPS listeners with untrusted certs).
# Applies to: health checks, scenario HTTP requests, cleanup requests.
if ($Spec.skip_ssl_verify -eq $true) {
    Write-Host "SSL certificate verification disabled (skip_ssl_verify: true)"
    try {
        add-type @"
using System.Net;
using System.Security.Cryptography.X509Certificates;
public class TrustAllCertsPolicy : ICertificatePolicy {
    public bool CheckValidationResult(
        ServicePoint srvPoint, X509Certificate certificate,
        WebRequest request, int certificateProblem) {
        return true;
    }
}
"@
    } catch {
        # Type may already exist from a previous invocation in the same process
    }
    [System.Net.ServicePointManager]::CertificatePolicy = [TrustAllCertsPolicy]::new()
}

# ============================================================================
# PHASE 0: Dependency Check
# ============================================================================
Write-Host ""
Write-Host "=== Phase 0: Dependency Check ==="

$depResult = CheckDependencies -SpecData $Spec
$PrereqCheck.dependencies_checked = $depResult.Checked

if (-not $depResult.Available) {
    $PrereqCheck.dependencies_available = $false
    $skipReason = "Dependency '$($depResult.FailedName)' unavailable at $($depResult.FailedHost):$($depResult.FailedPort)"
    Write-Host "Dependency check FAILED: $skipReason"
    $skipEvidence = BuildSkippedEvidence -Module $Module -YamlFile $YamlSpecAbsolute -SkipReason $skipReason -DependenciesChecked $depResult.Checked
    WriteEvidenceJson -Evidence $skipEvidence -OutputPath $EvidenceOutputPath
    exit 0
}
else {
    $PrereqCheck.dependencies_available = $true
    if ($depResult.Checked.Count -gt 0) {
        Write-Host "All $($depResult.Checked.Count) dependencies available"
    }
    else {
        Write-Host "No dependencies defined in YAML spec"
    }
}

# ============================================================================
# PHASE 1: GIVEN (Prerequisites)
# ============================================================================
Write-Host ""
Write-Host "=== Phase 1: Given (Prerequisites) ==="

# Normalize given to array
$GivenSteps = @()
if ($Spec.given -ne $null) {
    if ($Spec.given -is [array]) {
        $GivenSteps = $Spec.given
    } else {
        $GivenSteps = @($Spec.given)
    }
}

foreach ($givenStep in $GivenSteps) {
    # Build if build_command is defined in this given step
    $buildCommand = if ($givenStep.build_command -ne $null -and $givenStep.build_command -ne "") { $givenStep.build_command.ToString() } else { "" }
if ($buildCommand -ne "") {
    # ── seq_id substitution in build_command ──
    $buildCommand = $buildCommand -replace '\$\{seq_id\}', $GlobalSeqId
    $GlobalSeqId++
    Write-Host "Building: $buildCommand"

    try {
        $buildOutput = Invoke-Expression $buildCommand 2>&1
        $buildExitCode = $LASTEXITCODE

        if ($buildExitCode -ne 0) {
            Write-Host "Build FAILED with exit code $buildExitCode"
            Write-Host $buildOutput
            $PrereqCheck.build_success = $false

 # Build failure is not a SKIP scenario - it's a FAIL
            $failEvidence = @{
                metadata = @{
                    module        = $Module
                    yaml_file     = $YamlSpecAbsolute
                    executed_at   = $UTC_NOW
                    environment   = "PowerShell"
                    script_version = $SCRIPT_VERSION
                }
                prerequisite_check = $PrereqCheck
                tests    = @()
                cleanup  = @{
                    resources_deleted = @()
                    process_stopped   = $false
                }
                service_log = @{
                    service_logs = @{}
                }
                summary  = @{
                    total_tests    = 0
                    passed         = 0
                    failed         = 0
                    skipped        = 0
                    high_priority_failures = 1
                    low_priority_failures  = 0
                    blocking       = $true
                    overall_result = "FAIL"
                    skipped_reason = "Build failed: $buildCommand (exit code $buildExitCode)"
                }
            }
            WriteEvidenceJson -Evidence $failEvidence -OutputPath $EvidenceOutputPath
            exit 1
        }

        $PrereqCheck.build_success = $true
        Write-Host "Build succeeded"
    }
    catch {
        Write-Host "Build command execution error: $($_.Exception.Message)"
        $PrereqCheck.build_success = $false

$failEvidence = @{
            metadata = @{
                module        = $Module
                yaml_file     = $YamlSpecAbsolute
                executed_at   = $UTC_NOW
                environment   = "PowerShell"
                script_version = $SCRIPT_VERSION
            }
            prerequisite_check = $PrereqCheck
            tests    = @()
            cleanup  = @{
                resources_deleted = @()
                process_stopped   = $false
            }
            service_log = @{
                service_logs = @{}
            }
            summary  = @{
                total_tests    = 0
                passed         = 0
                failed         = 0
                skipped        = 0
                high_priority_failures = 1
                low_priority_failures  = 0
                blocking       = $true
                overall_result = "FAIL"
                skipped_reason = "Build command error: $($_.Exception.Message)"
            }
        }
        WriteEvidenceJson -Evidence $failEvidence -OutputPath $EvidenceOutputPath
        exit 1
    }
}
}

# If no build was required across all steps, mark as success
if ($PrereqCheck.build_success -eq $null) {
    $PrereqCheck.build_success = $true
    Write-Host "Build not required"
}

# ============================================================================
# PHASE 2: STARTUP
# ============================================================================
Write-Host ""
Write-Host "=== Phase 2: Service Startup ==="

try {
    foreach ($givenStep in $GivenSteps) {
    # ── startup_command: supports scalar string or array of strings ──
    $rawCmd = if ($givenStep.startup_command -ne $null) { $givenStep.startup_command } else { $null }
    $startupCmdList = @()
    if ($rawCmd -ne $null) {
        if ($rawCmd -is [array] -or ($rawCmd -is [System.Collections.IEnumerable] -and $rawCmd -isnot [string])) {
            $startupCmdList = @($rawCmd)
        }
        else {
            $startupCmdList = @($rawCmd.ToString())
        }
    }
    foreach ($startupCommand in $startupCmdList) {
        $startupCommand = $startupCommand.ToString()
        if ($startupCommand -eq "") { continue }
        # ── seq_id substitution: replace ${seq_id} with current counter, then increment ──
        $startupCommand = $startupCommand -replace '\$\{seq_id\}', $GlobalSeqId
        $GlobalSeqId++
        Write-Host "Starting service: $startupCommand"

        $startupStartTime = Get-Date

        # Parse the startup command to separate executable and arguments
        # Handle paths with spaces and arguments
        $commandTokens = $startupCommand -split ' '
        $executable = $commandTokens[0]
        $arguments = if ($commandTokens.Length -gt 1) { $commandTokens[1..($commandTokens.Length - 1)] -join ' ' } else { "" }

        # Resolve executable path if it's relative (contains ./ or .\ prefix)
        if ($executable -match '^\.\\|^\.\/') {
            $executable = $executable -replace '^\.\\|^\.\/', ''
            $executable = Join-Path (Split-Path -LiteralPath $YamlSpecAbsolute -Parent) $executable
        }

        # Start the process (stdout/stderr inherited to parent console UNLESS ServiceLogDir is set)
        $processParams = @{
            FilePath    = $executable
            ErrorAction = "Stop"
        }

        if ($arguments -ne "") {
            $processParams["ArgumentList"] = $arguments
        }

        # When ServiceLogDir is configured, redirect stdout+stderr to per-process log files
        # This allows the Agent to read service logs for post-mortem analysis.
        # NOTE: PowerShell 5.1 Start-Process does not allow RedirectStandardOutput and
        # RedirectStandardError to point to the same file, so two separate files are created:
        #   - <name>-<timestamp>.log       (stdout)
        #   - <name>-<timestamp>.err.log   (stderr)
        if ($ServiceLogDir -ne "") {
            $timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
            $exeName = [System.IO.Path]::GetFileNameWithoutExtension($executable)
            $stdoutLogFile = Join-Path $ServiceLogDir "${exeName}-${timestamp}.log"
            $stderrLogFile = Join-Path $ServiceLogDir "${exeName}-${timestamp}.err.log"

            $processParams["RedirectStandardOutput"] = $stdoutLogFile
            $processParams["RedirectStandardError"]  = $stderrLogFile
            $processParams["NoNewWindow"]           = $true

            Write-Host "  Service output will be logged to:"
            Write-Host "    stdout: $stdoutLogFile"
            Write-Host "    stderr: $stderrLogFile"
        }

        $proc = Start-Process @processParams -PassThru
        $ServiceProcesses += $proc
        $ServiceProcessIds += $proc.Id

        Write-Host "Service process started (PID: $($proc.Id))"

        # Record the log file paths for this PID (if log redirection is enabled)
        if ($ServiceLogDir -ne "") {
            $ServiceLogFiles[$proc.Id.ToString()] = @{
                stdout = $stdoutLogFile
                stderr = $stderrLogFile
            }
        }

        # Readiness probe: poll base_url until service is listening or timeout
        # If base_url is defined, perform HTTP health polling (handles slow startup reliably)
        # If not defined, fall back to a 2-second fixed delay for backward compatibility
        $ready = $true
        if ($GlobalBaseUrl -ne "") {
            # Read readiness config from YAML (optional section), with sensible defaults
            $readinessUrl      = $GlobalBaseUrl
            $readinessTimeout  = 60
            $readinessInterval = 2
            if ($Spec.readiness -ne $null) {
                if ($Spec.readiness.url -ne $null -and $Spec.readiness.url -ne "")       { $readinessUrl      = $Spec.readiness.url.ToString() }
                if ($Spec.readiness.timeout_seconds -ne $null)                            { $readinessTimeout  = [int]$Spec.readiness.timeout_seconds }
                if ($Spec.readiness.interval_seconds -ne $null)                           { $readinessInterval = [int]$Spec.readiness.interval_seconds }
            }
            $ready = WaitForServiceReadiness -ProbeUrl $readinessUrl -TimeoutSeconds $readinessTimeout -IntervalSeconds $readinessInterval
        }
        else {
            # No base_url defined: fallback fixed delay (best-effort, original behavior)
            Start-Sleep -Seconds 2
        }

        $duration = ((Get-Date) - $startupStartTime).TotalSeconds
        if ($ready) {
            $PrereqCheck.startup_success = $true
            Write-Host "Startup successful (${([math]::Round($duration, 1))}s)"
        }
        else {
            $PrereqCheck.startup_success = $false
            Write-Host "Startup FAILED: service did not become ready within timeout"
        }
        $PrereqCheck.startup_duration_seconds = [math]::Round($duration, 2)
    }
}

    # If no startup command was specified in any given step, assume service already running
    if ($PrereqCheck.startup_success -eq $false -and $ServiceProcesses.Count -eq 0) {
        Write-Host "No startup command specified in any given step - service assumed already running"
        $PrereqCheck.startup_success = $true
        $PrereqCheck.startup_duration_seconds = 0
    }

    # If readiness probe timed out, abort Phase 3 — no point testing an unreachable service.
    # Throwing here triggers catch (marks startup_failure as HIGH priority) + finally (teardown).
    if ($PrereqCheck.startup_success -eq $false) {
        throw "Service startup failed: readiness probe did not receive a response within timeout ($($PrereqCheck.startup_duration_seconds)s elapsed)"
    }

# ============================================================================
    # PHASE 3: SCENARIO EXECUTION
    # ============================================================================
    Write-Host ""
    Write-Host "=== Phase 3: Scenario Execution ==="

    # Read fail_fast from YAML (default: true). When enabled, any test FAIL
    # immediately aborts the scenario execution loop: all remaining scenarios
    # are SKIPped with reason "fail_fast: previous test 'X' failed".
    $FailFastEnabled = $true
    if ($Spec.fail_fast -ne $null) {
        # YAML may provide boolean false or string "false"
        if ($Spec.fail_fast -eq $false -or ($Spec.fail_fast -is [string] -and $Spec.fail_fast.ToString().ToLower() -eq "false")) {
            $FailFastEnabled = $false
        }
    }
    $FailFastTriggered = $false
    $FailFastTriggeringTest = ""
    Write-Host "fail_fast: $FailFastEnabled"

    # Build a map of test results for depends_on resolution
    $TestResultMap = @{}

    $Scenarios = $Spec.scenarios
    if ($Scenarios -ne $null) {
        foreach ($testDef in $Scenarios) {
            $testName = $testDef.name
            Write-Host ""
            Write-Host "Running test: $testName"


            # ── Detect scenario mode: Commands vs REST ──
            $isCommandsMode = $false
            $commandsList = @()
            $sceneMode = "rest"
            if ($testDef.when -ne $null -and $testDef.when -isnot [array] -and $testDef.when.commands -ne $null) {
                $isCommandsMode = $true
                $sceneMode = "commands"
                $rawCommands = $testDef.when.commands
                if ($rawCommands -is [array] -or ($rawCommands -is [System.Collections.IEnumerable] -and $rawCommands -isnot [string])) {
                    $commandsList = @($rawCommands)
                } else {
                    $commandsList = @($rawCommands.ToString())
                }
            }

            # ── Extract retry configuration (shared by both modes) ──
            $testThen = $testDef.then
            $testRetry = $testDef.retry
            $maxRetries = 0
            $retryInterval = 2
            if ($testRetry -ne $null) {
                if ($testRetry.max_retries -ne $null) {
                    $maxRetries = [int]$testRetry.max_retries
                }
                if ($testRetry.interval_seconds -ne $null) {
                    $retryInterval = [int]$testRetry.interval_seconds
                }
            }

            # ── Extract wait_seconds (pre-execution delay for async operations) ──
            $waitSeconds = 0
            if ($testDef.wait_seconds -ne $null) {
                $waitSeconds = [int]$testDef.wait_seconds
            }

            # ── Initialize variables used by shared result code (REST-specific get overwritten in REST branch) ──
            $testStatusCode = 0
            $testBodyContainsMatch = $null

            # ── fail_fast circuit breaker ──
            # When fail_fast is enabled AND an earlier test has failed, skip every
            # remaining scenario unconditionally (even those with no depends_on).
            # This avoids log noise that makes root-cause analysis difficult.
            if ($FailFastEnabled -and $FailFastTriggered) {
                $testPriority = if ($testDef.priority -ne $null) { $testDef.priority.ToString() } else { "LOW" }
                $skipReasonText = "fail_fast: previous test '$FailFastTriggeringTest' failed, remaining scenarios skipped"
                Write-Host "  SKIPPING: $skipReasonText"
                $testEntry = @{
                    name            = $testName
                    mode            = $sceneMode
                    result          = "SKIP"
                    priority        = $testPriority
                    status_code     = 0
                    body_contains_match = $null
                    duration_ms     = 0
                    error_message   = ""
                    skip_reason     = $skipReasonText
                }
                $TestResults += $testEntry
                $TestResultMap[$testName] = "SKIP"
                continue
            }


            if ($isCommandsMode) {
                # ── COMMANDS MODE EXECUTION ──
                if ($waitSeconds -gt 0) {
                    Write-Host "  Waiting ${waitSeconds}s before execution (async delay)..."
                    Start-Sleep -Seconds $waitSeconds
                }
                $testStartTime = Get-Date
                $testPassed = $false
                $testErrorMsg = ""
                $cmdOutputs = @()
                $cmdExitCodes = @()

                for ($attempt = 0; $attempt -le $maxRetries; $attempt++) {
                    if ($attempt -gt 0) {
                        Write-Host "  Retry attempt #$attempt (of $maxRetries), waiting $retryInterval seconds..."
                        Start-Sleep -Seconds $retryInterval
                    }

                    # Execute each command sequentially
                    $cmdOutputs = @()
                    $cmdExitCodes = @()
                    $cmdExecError = ""
                    $cmdExecFailed = $false

                    foreach ($cmd in $commandsList) {
                        $cmdStr = $cmd.ToString()
                        # Substitute ${base_url} in command
                        if ($GlobalBaseUrl -ne "" -and $cmdStr -match '\$\{base_url\}') {
                            $cmdStr = $cmdStr -replace '\$\{base_url\}', $GlobalBaseUrl
                        }
                        # ── seq_id substitution in commands ──
                        $cmdStr = $cmdStr -replace '\$\{seq_id\}', $GlobalSeqId
                        $GlobalSeqId++
                        Write-Host "  Executing command: $cmdStr"

                        try {
                            $cmdResult = Invoke-Expression $cmdStr 2>&1
                            $cmdExit = $LASTEXITCODE
                            if ($cmdExit -eq $null) { $cmdExit = 0 }
                            $cmdOutputStr = ($cmdResult | Out-String).TrimEnd()
                            $cmdOutputs += $cmdOutputStr
                            $cmdExitCodes += [int]$cmdExit
                            Write-Host "  Command exit code: $cmdExit"
                        }
                        catch {
                            Write-Host "  Command execution error: $($_.Exception.Message)"
                            $cmdExecError = $_.Exception.Message
                            $cmdOutputs += ""
                            $cmdExitCodes += 1
                            $cmdExecFailed = $true
                            break
                        }
                    }

                    if ($cmdExecFailed) {
                        $testErrorMsg = "Command execution error: $cmdExecError"
                        if ($attempt -lt $maxRetries) { continue }
                        break
                    }

                    # Validate assertions (1:1 with commands)
                    # Normalize then to array
                    $thenArray = @()
                    if ($testThen -ne $null) {
                        if ($testThen -is [array]) {
                            $thenArray = @($testThen)
                        } else {
                            $thenArray = @($testThen)
                        }
                    }

                    $allAssertionsPass = $true
                    $lastAssertionError = ""

                    # Check then count == commands count
                    if ($thenArray.Count -ne $commandsList.Count) {
                        $allAssertionsPass = $false
                        $lastAssertionError = "Assertion count ($($thenArray.Count)) does not match command count ($($commandsList.Count))"
                        Write-Host "  Assertion FAILED: $lastAssertionError"
                    } else {
                        for ($ai = 0; $ai -lt $thenArray.Count; $ai++) {
                            $assertion = $thenArray[$ai]
                            $output = if ($ai -lt $cmdOutputs.Count) { $cmdOutputs[$ai] } else { "" }
                            $exitCode = if ($ai -lt $cmdExitCodes.Count) { $cmdExitCodes[$ai] } else { 1 }

                            # ── result_code assertion (integer or array, OR logic) ──
                            if ($assertion.result_code -ne $null) {
                                $rcExpected = $assertion.result_code
                                $rcMatch = $false
                                if ($rcExpected -is [array]) {
                                    foreach ($rc in $rcExpected) {
                                        if ($exitCode -eq [int]$rc) { $rcMatch = $true; break }
                                    }
                                } else {
                                    $rcMatch = ($exitCode -eq [int]$rcExpected)
                                }
                                Write-Host "  Command[$ai] result_code: $exitCode (expected: $rcExpected) - Match: $rcMatch"
                                if (-not $rcMatch) {
                                    $allAssertionsPass = $false
                                    $lastAssertionError = "Command[$ai] result_code mismatch: got $exitCode, expected $rcExpected"
                                    Write-Host "  Assertion FAILED: $lastAssertionError"
                                    break
                                }
                            }

                            # ── contains assertion (string array, AND logic, case-insensitive) ──
                            if ($assertion.contains -ne $null) {
                                $containsList = @($assertion.contains)
                                foreach ($substr in $containsList) {
                                    $found = $output.ToLower().Contains($substr.ToString().ToLower())
                                    Write-Host "  Command[$ai] contains '$substr': $found"
                                    if (-not $found) {
                                        $allAssertionsPass = $false
                                        $lastAssertionError = "Command[$ai] output does not contain '$substr'"
                                        Write-Host "  Assertion FAILED: $lastAssertionError"
                                        break
                                    }
                                }
                                if (-not $allAssertionsPass) { break }
                            }

                            # ── not_contains assertion (string array, AND logic, case-insensitive) ──
                            if ($assertion.not_contains -ne $null) {
                                $notContainsList = @($assertion.not_contains)
                                foreach ($substr in $notContainsList) {
                                    $absent = -not $output.ToLower().Contains($substr.ToString().ToLower())
                                    Write-Host "  Command[$ai] not_contains '$substr': $absent"
                                    if (-not $absent) {
                                        $allAssertionsPass = $false
                                        $lastAssertionError = "Command[$ai] output contains unexpected '$substr'"
                                        Write-Host "  Assertion FAILED: $lastAssertionError"
                                        break
                                    }
                                }
                                if (-not $allAssertionsPass) { break }
                            }
                        }
                    }

                    if ($allAssertionsPass) {
                        $testPassed = $true
                        break
                    } else {
                        $testErrorMsg = $lastAssertionError
                        if ($attempt -lt $maxRetries) { continue }
                        break
                    }
                }

            } else {
            # ── REST MODE EXECUTION ──
# Normalize "when" to array (supports both array and single object)
            $whenSteps = @()
            if ($testDef.when -ne $null) {
                if ($testDef.when -is [array]) {
                    $whenSteps = $testDef.when
                } else {
                    $whenSteps = @($testDef.when)
                }
            }

            # Execute the test (with retry logic)
            $testStartTime = Get-Date
            $testPassed = $false
            $testStatusCode = 0
            $testResponseBody = ""
            $testErrorMsg = ""
            $testBodyContainsMatch = $null

            if ($waitSeconds -gt 0) {
                Write-Host "  Waiting ${waitSeconds}s before execution (async delay)..."
                Start-Sleep -Seconds $waitSeconds
            }

            for ($attempt = 0; $attempt -le $maxRetries; $attempt++) {
                if ($attempt -gt 0) {
                    Write-Host "  Retry attempt #$attempt (of $maxRetries), waiting $retryInterval seconds..."
                    Start-Sleep -Seconds $retryInterval
                }

                # ── Iterate through when steps (send all HTTP requests in order) ──
                $whenStepFailed = $false
                foreach ($whenStep in $whenSteps) {
                    $stepMethod = if ($whenStep.method -ne $null) { $whenStep.method } else { "GET" }
                    $stepUrl = if ($whenStep.url -ne $null) { $whenStep.url.ToString() } else { "" }
                    # Substitute ${base_url} in step URL
                    if ($GlobalBaseUrl -ne "" -and $stepUrl -match '\$\{base_url\}') {
                        $stepUrl = $stepUrl -replace '\$\{base_url\}', $GlobalBaseUrl
                    }
                    # ── seq_id substitution in REST URL ──
                    $stepUrl = $stepUrl -replace '\$\{seq_id\}', $GlobalSeqId
                    $GlobalSeqId++
                    $stepHeaders = @{}
                    if ($whenStep.headers -ne $null) {
                        $headerObj = ConvertToNestedHashtable -InputObject $whenStep.headers
                        foreach ($key in $headerObj.Keys) {
                            $stepHeaders[$key] = $headerObj[$key]
                        }
                    }
                    $stepBody = if ($whenStep.body -ne $null) { $whenStep.body.ToString() } else { "" }

                    Write-Host "  Executing: $stepMethod $stepUrl"

                    $httpResult = InvokeSafeWebRequest -Url $stepUrl -Method $stepMethod -Headers $stepHeaders -Body $stepBody -TimeoutSeconds 30

                    if (-not $httpResult.Success) {
                        $testErrorMsg = $httpResult.ErrorMessage
                        Write-Host "  HTTP request failed: $testErrorMsg"
                        $whenStepFailed = $true
                        break
                    }

                    # Track last response (for then assertions)
                    $testStatusCode = $httpResult.StatusCode
                    $testResponseBody = $httpResult.Body
                }

                if ($whenStepFailed) {
                    if ($attempt -lt $maxRetries) {
                        continue
                    }
                    break
                }

                # Validate all assertions in the "then" array (AND logic: ALL must pass)
                $allAssertionsPass = $true
                $lastAssertionError = ""
                $testBodyContainsMatch = $null

                if ($testThen -ne $null) {
                    # Normalize then to array (YAML single item may deserialize as PSCustomObject, not array)
                    $thenArray = @($testThen)
                    if ($testThen -is [array]) {
                        $thenArray = @($testThen)
                    }
                    elseif ($testThen -is [PSCustomObject]) {
                        $thenArray = @($testThen)
                    }

                    foreach ($assertion in $thenArray) {
                        # ── status assertion ──
                        if ($assertion.status -ne $null) {
                            $statusMatch = ValidateStatusCode -ReceivedCode $testStatusCode -ExpectedStatus $assertion.status
                            Write-Host "  Status: $testStatusCode (expected: $($assertion.status)) - Match: $statusMatch"

                            if (-not $statusMatch) {
                                $allAssertionsPass = $false
                                $lastAssertionError = "Status code mismatch: got $testStatusCode, expected $($assertion.status)"
                                Write-Host "  Assertion FAILED: $lastAssertionError"
                                break
                            }
                        }

                        # ── body assertion (strict JSON map match) ──
                        if ($assertion.body -ne $null) {
                            $bodyMatch = ValidateBodyStrict -ResponseBody $testResponseBody -ExpectedBody $assertion.body
                            # Reuse body_contains_match tracking for evidence output
                            if ($testBodyContainsMatch -eq $null) {
                                $testBodyContainsMatch = $bodyMatch
                            }
                            elseif ($testBodyContainsMatch -eq $true) {
                                $testBodyContainsMatch = $bodyMatch
                            }
                            Write-Host "  Body strict match check: $bodyMatch"

                            if (-not $bodyMatch) {
                                $allAssertionsPass = $false
                                $lastAssertionError = "Body strict match failed: expected map key-value pairs not found in response"
                                Write-Host "  Assertion FAILED: $lastAssertionError"
                                break
                            }
                        }

                        # ── body_contains assertion ──
                        if ($assertion.body_contains -ne $null) {
                            $bodyMatch = ValidateBodyContains -ResponseBody $testResponseBody -ExpectedSubstrings $assertion.body_contains
                            # Track body_contains result for evidence (true if ANY body_contains check passes)
                            if ($testBodyContainsMatch -eq $null) {
                                $testBodyContainsMatch = $bodyMatch
                            }
                            elseif ($testBodyContainsMatch -eq $true) {
                                $testBodyContainsMatch = $bodyMatch
                            }
                            Write-Host "  Body contains check: $bodyMatch"

                            if (-not $bodyMatch) {
                                $allAssertionsPass = $false
                                $lastAssertionError = "Body does not contain expected substring(s)"
                                Write-Host "  Assertion FAILED: $lastAssertionError"
                                break
                            }
                        }

                        # ── body_not_contains assertion ──
                        if ($assertion.body_not_contains -ne $null) {
                            $bodyNotMatch = ValidateBodyNotContains -ResponseBody $testResponseBody -ExpectedAbsentSubstrings $assertion.body_not_contains
                            Write-Host "  Body not-contains check: $bodyNotMatch"

                            if (-not $bodyNotMatch) {
                                $allAssertionsPass = $false
                                $lastAssertionError = "Body contains unexpected substring(s)"
                                Write-Host "  Assertion FAILED: $lastAssertionError"
                                break
                            }
                        }

                        # ── body_json_path assertion ──
                        # Supports two formats:
                        #   1. Single path: body_json_path: "$.key" + body_value: "expected"
                        #   2. Map format:  body_json_path: { "$.key1": "val1", "$.key2": "val2" }
                        if ($assertion.body_json_path -ne $null) {
                            $jpObj = $assertion.body_json_path

                            # Determine format: string (single path) or map (multiple paths)
                            $jpIsString = ($jpObj -is [string])

                            # Build a list of (path, expectedValue) pairs
                            $jpPairs = @()
                            if ($jpIsString) {
                                $jpPairs += @{ path = $jpObj; expected = $assertion.body_value }
                            }
                            else {
                                # Map format: keys are paths, values are expected
                                $jpMap = @{}
                                if ($jpObj -is [hashtable]) {
                                    $jpMap = $jpObj
                                }
                                elseif ($jpObj -is [PSCustomObject]) {
                                    foreach ($prop in $jpObj.PSObject.Properties) {
                                        $jpMap[$prop.Name] = $prop.Value
                                    }
                                }
                                foreach ($entry in $jpMap.GetEnumerator()) {
                                    $jpPairs += @{ path = $entry.Key; expected = $entry.Value }
                                }
                            }

                            foreach ($pair in $jpPairs) {
                                $pathStr = $pair.path
                                $expectedVal = $pair.expected
                                $actualVal = ExtractJsonValue -ResponseBody $testResponseBody -JsonPath $pathStr
                                $valMatch = ($actualVal -eq ($expectedVal.ToString()))
                                Write-Host "  JSON path '$pathStr' value check: actual='$actualVal' expected='$expectedVal' - Match: $valMatch"

                                if (-not $valMatch) {
                                    $allAssertionsPass = $false
                                    $lastAssertionError = "JSON path '$pathStr' value mismatch: got '$actualVal', expected '$expectedVal'"
                                    Write-Host "  Assertion FAILED: $lastAssertionError"
                                    break
                                }
                            }

                            if (-not $allAssertionsPass) { break }
                        }
                    }
                }
                else {
                    # No then assertions defined - test passes if HTTP request succeeded
                    Write-Host "  No assertions defined - HTTP request succeeded, test passes"
                    $allAssertionsPass = $true
                }

                if ($allAssertionsPass) {
                    $testPassed = $true
                    break
                }
                else {
                    $testErrorMsg = $lastAssertionError
                    if ($attempt -lt $maxRetries) {
                        continue
                    }
                    break
                }
            }

            $testDuration = [math]::Round(((Get-Date) - $testStartTime).TotalMilliseconds, 0)

# Determine test result
            $testResultValue = if ($testPassed) { "PASS" } else { "FAIL" }
            $testPriority = if ($testDef.priority -ne $null) { $testDef.priority.ToString() } else { "LOW" }

            $testEntry = @{
                name            = $testName
                mode            = $sceneMode
                result          = $testResultValue
                priority        = $testPriority
                status_code     = $testStatusCode
                body_contains_match = $testBodyContainsMatch
                duration_ms     = $testDuration
                error_message   = $testErrorMsg
                skip_reason     = ""
            }
            $TestResults += $testEntry
            $TestResultMap[$testName] = $testResultValue

            Write-Host "  Result: $testResultValue (${testDuration}ms)"

            # ── trigger fail_fast ──
            # If this test FAILs and fail_fast is enabled, arm the circuit breaker
            # so all subsequent scenarios will be SKIPped in the next iteration.
            if ($testResultValue -eq "FAIL" -and $FailFastEnabled -and -not $FailFastTriggered) {
                $FailFastTriggered = $true
                $FailFastTriggeringTest = $testName
                Write-Host "  [fail_fast] armed: remaining scenarios will be skipped"
            }

            # Cleanup on failure: execute cleanup requests
            # cleanup_on_failure removed: scenarios should handle their own state
            }  # end else (REST mode)
        }
    }
    else {
        Write-Host "No scenarios defined in the YAML spec"
    }

    # ============================================================================
    # PHASE 4: CLEANUP
    # ============================================================================
    Write-Host ""
    Write-Host "=== Phase 4: Cleanup ==="

    $CleanupSection = $Spec.cleanup

    if ($CleanupSection -ne $null) {
        # Execute cleanup commands (generic shell commands, results not validated)
        $cleanupCommands = $CleanupSection.commands
        if ($null -ne $cleanupCommands) {
            # Normalize to array
            $cmdList = @()
            if ($cleanupCommands -is [array] -or ($cleanupCommands -is [System.Collections.IEnumerable] -and $cleanupCommands -isnot [string])) {
                $cmdList = @($cleanupCommands)
            }
            else {
                $cmdList = @($cleanupCommands.ToString())
            }

            foreach ($cmd in $cmdList) {
                $cmdStr = $cmd.ToString().Trim()
                if ($cmdStr -eq "") { continue }
                # ── seq_id substitution in cleanup commands ──
                $cmdStr = $cmdStr -replace '\$\{seq_id\}', $GlobalSeqId
                $GlobalSeqId++
                Write-Host "Cleanup command: $cmdStr"
                try {
                    Invoke-Expression $cmdStr 2>&1 | Out-Null
                }
                catch {
                    # Ignore errors - cleanup commands are not validated
                }
            }
        }

        # Stop all service processes if requested
        if ($CleanupSection.stop_process -eq $true -and $ServiceProcesses.Count -gt 0) {
            Write-Host "Stopping $($ServiceProcesses.Count) service process(es)..."
            foreach ($procId in $ServiceProcessIds) {
                try {
                    Stop-Process -Id $procId -Force -ErrorAction Stop
                    Start-Sleep -Seconds 2
                    $stillRunning = Get-Process -Id $procId -ErrorAction SilentlyContinue
                    if ($stillRunning -ne $null) {
                        Write-Host "  Process $procId still running, retrying kill..."
                        Stop-Process -Id $procId -Force -ErrorAction SilentlyContinue
                        Start-Sleep -Seconds 3
                    }
                    Write-Host "  Process $procId stopped"
                }
                catch {
                    Write-Host "  Error stopping process ${procId}: $($_.Exception.Message)"
                    try {
                        & taskkill /F /PID $procId 2>&1 | Out-Null
                        Start-Sleep -Seconds 2
                        Write-Host "  Process $procId killed via taskkill"
                    }
                    catch {
                        Write-Host "  Could not stop process $procId"
                    }
                }
            }
            $CleanupResult.process_stopped = $true
        }
        elseif ($CleanupSection.stop_process -ne $true) {
            Write-Host "Process stop not requested - leaving service running"
            $CleanupResult.process_stopped = $false
        }
    }
    else {
        # No cleanup section defined - default: stop all processes if we started any
        if ($ServiceProcesses.Count -gt 0) {
            Write-Host "No cleanup section defined - stopping $($ServiceProcesses.Count) service process(es) as default..."
            foreach ($procId in $ServiceProcessIds) {
                try {
                    Stop-Process -Id $procId -Force -ErrorAction SilentlyContinue
                    Start-Sleep -Seconds 2
                    Write-Host "  Process $procId stopped"
                }
                catch {
                    Write-Host "  Could not stop process ${procId}: $($_.Exception.Message)"
                }
            }
            $CleanupResult.process_stopped = $true
        }
    }

# ============================================================================
    # COMPUTE SUMMARY
    # ============================================================================
    $totalTests = $TestResults.Count
    $passedCount = 0
    $failedCount = 0
    $skippedCount = 0
    $highPriorityFailures = 0
    $lowPriorityFailures = 0

    foreach ($tr in $TestResults) {
        switch ($tr.result) {
            "PASS"  { $passedCount++ }
            "FAIL"  {
                $failedCount++
                if ($tr.priority -eq "HIGH") { $highPriorityFailures++ }
                else { $lowPriorityFailures++ }
            }
            "SKIP"  { $skippedCount++ }
        }
    }

    # Determine prerequisite_check priority
    $prereqPriority = "LOW"
    if ($PrereqCheck.build_success -eq $false) { $prereqPriority = "HIGH" }
    if ($PrereqCheck.startup_success -eq $false) { $prereqPriority = "HIGH" }
    $PrereqCheck.priority = $prereqPriority

    # Determine blocking: true if any HIGH priority failure (prerequisite or test)
    $isBlocking = ($highPriorityFailures -gt 0) -or ($prereqPriority -eq "HIGH")

    if ($failedCount -gt 0) {
        $OverallResult = "FAIL"
        $ExitCode = 1
    }
    elseif ($totalTests -eq 0 -or ($passedCount -eq 0 -and $skippedCount -eq $totalTests)) {
        $OverallResult = "SKIPPED"
        $SkippedReason = "No tests executed"
        $ExitCode = 0
    }
    else {
        $OverallResult = "PASS"
        $ExitCode = 0
    }

    Write-Host ""
    Write-Host "=== Summary ==="
    Write-Host "Total: $totalTests | Passed: $passedCount | Failed: $failedCount | Skipped: $skippedCount"
    Write-Host "Overall result: $OverallResult"
}
catch {
    Write-Host ""
    Write-Host "=== ERROR DURING EXECUTION ==="
    Write-Host "Error: $($_.Exception.Message)"
    Write-Host "Stack trace: $($_.ScriptStackTrace)"

# Compute partial summary
    $totalTests = $TestResults.Count
    $passedCount = 0
    $failedCount = 0
    $skippedCount = 0
    $highPriorityFailures = 0
    $lowPriorityFailures = 0

    foreach ($tr in $TestResults) {
        switch ($tr.result) {
            "PASS"  { $passedCount++ }
            "FAIL"  {
                $failedCount++
                if ($tr.priority -eq "HIGH") { $highPriorityFailures++ }
                else { $lowPriorityFailures++ }
            }
            "SKIP"  { $skippedCount++ }
        }
    }

    # Determine prerequisite_check priority for error case
    $prereqPriority = "HIGH"  # Error during execution implies HIGH priority
    if ($PrereqCheck.build_success -eq $false) { $prereqPriority = "HIGH" }
    if ($PrereqCheck.startup_success -eq $false) { $prereqPriority = "HIGH" }
    $PrereqCheck.priority = $prereqPriority

    $isBlocking = $true  # Error case is always blocking

    $OverallResult = "FAIL"
    $SkippedReason = "Execution error: $($_.Exception.Message)"
    $ExitCode = 1
}
finally {
    # ============================================================================
    # ALWAYS TEARDOWN - even on error
    # ============================================================================
    Write-Host ""
    Write-Host "=== Teardown (always) ==="

    # If process was started and not yet stopped, force-stop them all
    if ($ServiceProcesses.Count -gt 0 -and -not $CleanupResult.process_stopped) {
        Write-Host "Force-stopping $($ServiceProcesses.Count) service process(es) in teardown..."
        foreach ($procId in $ServiceProcessIds) {
            try {
                $processExists = Get-Process -Id $procId -ErrorAction SilentlyContinue
                if ($processExists -ne $null) {
                    Stop-Process -Id $procId -Force -ErrorAction SilentlyContinue
                    Start-Sleep -Seconds 2
                    $processStillExists = Get-Process -Id $procId -ErrorAction SilentlyContinue
                    if ($processStillExists -ne $null) {
                        & taskkill /F /PID $procId 2>&1 | Out-Null
                        Start-Sleep -Seconds 2
                    }
                    Write-Host "  Process $procId stopped in teardown"
                }
                else {
                    Write-Host "  Process $procId already terminated"
                }
            }
            catch {
                Write-Host "  Teardown: could not verify/stop process ${procId}: $($_.Exception.Message)"
            }
        }
        $CleanupResult.process_stopped = $true
    }

    # ============================================================================
    # WRITE FINAL EVIDENCE
    # ============================================================================
$finalEvidence = @{
        metadata = @{
            module        = $Module
            yaml_file     = $YamlSpecAbsolute
            executed_at   = $UTC_NOW
            environment   = "PowerShell"
            script_version = $SCRIPT_VERSION
        }
        prerequisite_check = $PrereqCheck
        tests    = $TestResults
        cleanup  = $CleanupResult
        service_log = @{
            service_logs = $ServiceLogFiles
        }
        summary  = @{
            total_tests    = $totalTests
            passed         = $passedCount
            failed         = $failedCount
            skipped        = $skippedCount
            high_priority_failures = $highPriorityFailures
            low_priority_failures  = $lowPriorityFailures
            blocking       = $isBlocking
            overall_result = $OverallResult
            skipped_reason = $SkippedReason
        }
    }

    WriteEvidenceJson -Evidence $finalEvidence -OutputPath $EvidenceOutputPath

    Write-Host ""
    Write-Host "E2E execution complete. Exit code: $ExitCode"
}

exit $ExitCode
