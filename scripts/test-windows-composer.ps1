param([Parameter(Mandatory=$true)][string]$AppPath)

# Run in an interactive Windows desktop after building the app. The isolated
# profile never reads or resets the person's account or clipboard.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms
$RunId = 'composer-' + [Guid]::NewGuid().ToString('N')
$DataDir = Join-Path $env:TEMP ('iris-' + $RunId)
New-Item -ItemType Directory -Path $DataDir | Out-Null
$PreviousRunId = $env:IRIS_UI_TEST_RUN_ID
$PreviousDataDir = $env:IRIS_UI_TEST_DATA_DIR
$App = $null

function Wait-For([scriptblock]$Condition, [string]$Description) {
    $Until = [DateTime]::UtcNow.AddSeconds(30)
    do {
        $Result = & $Condition
        if ($Result) { return $Result }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $Until)
    if ($Window) {
        $Window.FindAll([System.Windows.Automation.TreeScope]::Descendants,
            [System.Windows.Automation.Condition]::TrueCondition) | Select-Object -First 60 | ForEach-Object {
            Write-Host "UI: $($_.Current.ControlType.ProgrammaticName) id=$($_.Current.AutomationId) name=$($_.Current.Name)"
        }
    }
    throw "Timed out: $Description"
}

function Find-Id([string]$Id) {
    $Window.FindFirst([System.Windows.Automation.TreeScope]::Descendants,
        [System.Windows.Automation.PropertyCondition]::new(
            [System.Windows.Automation.AutomationElement]::AutomationIdProperty, $Id))
}

function Invoke-Id([string]$Id) {
    $Element = Wait-For { Find-Id $Id } $Id
    $Pattern = $Element.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
    $Pattern.Invoke()
}

function Type-Into([string]$Id, [string]$Text) {
    $Element = Wait-For { Find-Id $Id } $Id
    $Element.SetFocus()
    [System.Windows.Forms.SendKeys]::SendWait($Text)
}

try {
    $env:IRIS_UI_TEST_RUN_ID = $RunId
    $env:IRIS_UI_TEST_DATA_DIR = $DataDir
    $App = Start-Process -FilePath (Resolve-Path $AppPath) -PassThru
    $Window = Wait-For {
        $App.Refresh()
        if ($App.HasExited) { throw 'App exited before the typing test' }
        if ($App.MainWindowHandle -ne 0) {
            [System.Windows.Automation.AutomationElement]::FromHandle($App.MainWindowHandle)
        }
    } 'app window'
    Invoke-Id 'CreateButton'
    Type-Into 'NameInput' 'Keyboard test'
    Invoke-Id 'CreateButton'
    $Group = Wait-For {
        $Window.FindFirst([System.Windows.Automation.TreeScope]::Descendants,
            [System.Windows.Automation.AndCondition]::new(
                [System.Windows.Automation.PropertyCondition]::new(
                    [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
                    [System.Windows.Automation.ControlType]::Button),
                [System.Windows.Automation.PropertyCondition]::new(
                    [System.Windows.Automation.AutomationElement]::NameProperty, 'Group')))
    } 'new group button'
    $Group.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
    # A fresh throwaway public key makes a test-only group with no real contact.
    $Key = [System.Security.Cryptography.ECDsa]::Create(
        [System.Security.Cryptography.ECCurve]::CreateFromFriendlyName('secP256k1'))
    try {
        $Peer = ([BitConverter]::ToString($Key.ExportParameters($false).Q.X)).Replace('-', '').ToLowerInvariant()
    } finally { $Key.Dispose() }
    Type-Into 'MemberSearchInput' $Peer
    Invoke-Id 'AddMemberButton'
    Invoke-Id 'NextButton' 
    Type-Into 'NameInput' 'Keyboard notes'
    Invoke-Id 'CreateButton'
    $Input = Wait-For { Find-Id 'Input' } 'message input'
    $Input.SetFocus()
    $Expected = ''
    foreach ($Character in 'hello from keyboard'.ToCharArray()) {
        # Send to the focused control, never refocus between characters.
        [System.Windows.Forms.SendKeys]::SendWait([string]$Character)
        $Expected += $Character
        Wait-For {
            $Current = Find-Id 'Input'
            $Value = $Current.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).Current.Value
            if (!$Current.Current.HasKeyboardFocus) { throw 'Message input lost keyboard focus' }
            $Value -eq $Expected
        } "typing $Expected" | Out-Null
        Start-Sleep -Milliseconds 150
    }
    [System.Windows.Forms.SendKeys]::SendWait('+{ENTER}')
    [System.Windows.Forms.SendKeys]::SendWait('second line')
    $Expected += "`r`nsecond line"
    Wait-For {
        (Find-Id 'Input').GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).Current.Value -eq $Expected
    } 'Shift+Enter adds a newline' | Out-Null
    [System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
    Wait-For {
        (Find-Id 'Input').GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).Current.Value -eq ''
    } 'composer clears after sending' | Out-Null
    Wait-For {
        $Window.FindFirst([System.Windows.Automation.TreeScope]::Descendants,
            [System.Windows.Automation.PropertyCondition]::new(
                [System.Windows.Automation.AutomationElement]::NameProperty, $Expected))
    } 'sent message in chat' | Out-Null
    Write-Output 'PASS: Windows keyboard typing retains focus and sends the complete message'
} finally {
    if ($App -and !$App.HasExited) {
        $App.CloseMainWindow() | Out-Null
        if (!$App.WaitForExit(5000)) { $App.Kill(); $App.WaitForExit() }
    }
    $env:IRIS_UI_TEST_RUN_ID = $PreviousRunId
    $env:IRIS_UI_TEST_DATA_DIR = $PreviousDataDir
    Remove-Item -LiteralPath $DataDir -Recurse -Force
}
