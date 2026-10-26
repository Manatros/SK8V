# Builds an emissive copy `<name>_glow` (.ydr + .ytyp) of every board shop
# wheel part (skatev_w_*), for glowing wheels (board_parts SetGlowWheels).
# The copy keeps the wheel's geometry, skinning (the board's seven bones) and
# its own colour texture; the shader is GTA's emissive.sps with its full
# parameter set (order and defaults as GTA declares them), and the vertices
# drop the tangent the emissive layout does not have. GTA must be closed
# before -Install copies the result into the parts folder.
param(
    [Parameter(Mandatory = $true)][string]$Parts,  # private\board-parts
    [Parameter(Mandatory = $true)][string]$Out,    # staging folder
    [string]$CodeWalker = 'C:\SkateMods\tools\CodeWalker',
    [double]$Emissive = 3.0,
    [string[]]$Only,
    # Fixed glow colours (host GlowColours): `<name>_glow_<colour>` copies whose
    # colour texture is the wheel's own brightness in that colour (rc7 test: a
    # Red glow on white wheels still glowed white).
    [switch]$Colours,
    [switch]$Install
)
$glowColours = [ordered]@{ white = @(255, 255, 255); red = @(255, 40, 30); orange = @(255, 120, 20); yellow = @(255, 220, 40)
    green = @(60, 255, 60); cyan = @(40, 230, 255); blue = @(40, 90, 255); purple = @(170, 60, 255); pink = @(255, 70, 190) }
$ErrorActionPreference = 'Stop'
foreach ($dll in 'SharpDX.dll', 'SharpDX.Mathematics.dll', 'CodeWalker.Core.dll') {
    [void][Reflection.Assembly]::LoadFrom((Join-Path $CodeWalker $dll))
}
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$inv = [Globalization.CultureInfo]::InvariantCulture

function Read-Xml([string]$path, [string]$work) {
    $bytes = [IO.File]::ReadAllBytes($path)
    $entry = [CodeWalker.GameFiles.RpfFile]::CreateResourceFileEntry([ref]$bytes, 0)
    $entry.Name = [IO.Path]::GetFileName($path); $entry.NameLower = $entry.Name.ToLowerInvariant()
    $data = [CodeWalker.GameFiles.ResourceBuilder]::Decompress($bytes)
    $name = ''
    New-Item -ItemType Directory -Force -Path $work | Out-Null
    [xml][CodeWalker.GameFiles.MetaXml]::GetXml($entry, $data, [ref]$name, $work)
}
function Write-Resource([xml]$doc, [string]$fileName, [string]$textures, [string]$to) {
    $trim = 0
    $format = [CodeWalker.GameFiles.XmlMeta]::GetXMLFormat("$fileName.xml", [ref]$trim)
    $bytes = [CodeWalker.GameFiles.XmlMeta]::GetData($doc, $format, $textures)
    if (-not $bytes) { throw "CodeWalker wrote nothing for $fileName" }
    [IO.File]::WriteAllBytes($to, $bytes)
}
function Param([xml]$doc, [string]$name, [string]$type, [hashtable]$v) {
    $p = $doc.CreateElement('Item'); $p.SetAttribute('name', $name); $p.SetAttribute('type', $type)
    foreach ($k in 'x', 'y', 'z', 'w') { if ($v.ContainsKey($k)) { $p.SetAttribute($k, $v[$k].ToString($inv)) } }
    $p
}

$wheels = Get-ChildItem -LiteralPath $Parts -Filter 'skatev_w_*.ydr' | Where-Object { $_.BaseName -notlike '*_glow*' }
if ($Only) { $wheels = $wheels | Where-Object { $Only -contains $_.BaseName } }
$done = 0
foreach ($ydr in $wheels) {
    $name = $ydr.BaseName; $glow = "${name}_glow"
    $work = Join-Path $Out "work\$name"
    $doc = Read-Xml $ydr.FullName $work
    $doc.Drawable.Name = $glow
    foreach ($shader in $doc.SelectNodes('/Drawable/ShaderGroup/Shaders/Item')) {
        $diffuse = $shader.SelectSingleNode('Parameters/Item[@name="DiffuseSampler"]')
        if (-not $diffuse) { throw "$name has a shader without DiffuseSampler" }
        $diffuse = $diffuse.CloneNode($true)
        $shader.Name = 'emissive'; $shader.FileName = 'emissive.sps'
        $params = $shader.SelectSingleNode('Parameters'); $params.RemoveAll()
        # GTA's emissive.sps parameters, in its declared order.
        [void]$params.AppendChild($diffuse)
        [void]$params.AppendChild((Param $doc 'matMaterialColorScale' 'Vector' @{ x = 1.0; y = 0.0; z = 0.0; w = 1.0 }))
        [void]$params.AppendChild((Param $doc 'HardAlphaBlend' 'Vector' @{ x = 1.0; y = 0.0; z = 0.0; w = 0.0 }))
        [void]$params.AppendChild((Param $doc 'useTessellation' 'Vector' @{ x = 0.0; y = 0.0; z = 0.0; w = 0.0 }))
        [void]$params.AppendChild((Param $doc 'emissiveMultiplier' 'Vector' @{ x = $Emissive; y = 0.0; z = 0.0; w = 0.0 }))
        [void]$params.AppendChild((Param $doc 'globalAnimUV1' 'Vector' @{ x = 0.0; y = 1.0; z = 0.0; w = 0.0 }))
        [void]$params.AppendChild((Param $doc 'globalAnimUV0' 'Vector' @{ x = 1.0; y = 0.0; z = 0.0; w = 0.0 }))
    }
    # The emissive layout has no tangent: drop it from every vertex buffer.
    foreach ($vb in $doc.SelectNodes('//VertexBuffer')) {
        $layout = $vb.SelectSingleNode('Layout')
        $names = @($layout.ChildNodes | ForEach-Object { $_.Name })
        $at = [array]::IndexOf($names, 'Tangent')
        if ($at -lt 0) { continue }
        [void]$layout.RemoveChild($layout.ChildNodes[$at])
        foreach ($data in $vb.SelectNodes('Data|Data1|Data2')) {
            $lines = $data.InnerText.Trim() -split "`r?`n"
            $data.InnerText = "`n" + (($lines | ForEach-Object {
                $groups = $_.Trim() -split '   '
                if ($groups.Count -ne $names.Count) { throw "$name`: vertex line with $($groups.Count) groups, layout has $($names.Count)" }
                (@(for ($i = 0; $i -lt $groups.Count; $i++) { if ($i -ne $at) { $groups[$i] } })) -join '   '
            }) -join "`n") + "`n"
        }
    }
    # CodeWalker exports the embedded textures into a folder named after the model.
    Write-Resource $doc "$glow.ydr" (Join-Path $work $name) (Join-Path $Out "$glow.ydr")
    # The archetype: same model under the new name.
    $ytypPath = Join-Path $Parts "$name.ytyp"
    $typ = Read-Xml $ytypPath (Join-Path $work 'ytyp')
    # Names appear as text or, unknown to CodeWalker, as their hash.
    $hash = 'hash_{0:X8}' -f [CodeWalker.GameFiles.JenkHash]::GenHash($name)
    $xml = $typ.OuterXml.Replace(">$name<", ">$glow<").Replace(">$hash<", ">$glow<")
    if ($xml -eq $typ.OuterXml) { throw "$name.ytyp does not name $name" }
    Write-Resource ([xml]$xml) "$glow.ytyp" (Join-Path $work 'ytyp') (Join-Path $Out "$glow.ytyp")
    if ($Colours) {
        $textures = Join-Path $work $name
        $diffuseNames = @($doc.SelectNodes('/Drawable/ShaderGroup/Shaders/Item/Parameters/Item[@name="DiffuseSampler"]/Name') | ForEach-Object { $_.InnerText }) | Select-Object -Unique
        foreach ($c in $glowColours.Keys) {
            $variant = "${glow}_$c"
            $tinted = Join-Path $work "tint_$c"
            if (Test-Path -LiteralPath $tinted) { Remove-Item -LiteralPath $tinted -Recurse -Force }
            Copy-Item -LiteralPath $textures -Destination $tinted -Recurse
            $vdoc = [xml]$doc.OuterXml
            $vdoc.Drawable.Name = $variant
            foreach ($texName in $diffuseNames) {
                $item = $vdoc.SelectSingleNode("/Drawable/ShaderGroup/TextureDictionary/Item[Name='$texName']")
                if (-not $item) { continue }
                $dds = Join-Path $tinted $item.FileName
                $tex = [CodeWalker.Utils.DDSIO]::GetTexture([IO.File]::ReadAllBytes($dds))
                $px = [CodeWalker.Utils.DDSIO]::GetPixels($tex, 0)
                $rgb = $glowColours[$c]
                $buf = New-Object byte[] $px.Length
                for ($i = 0; $i -lt $px.Length; $i += 4) {
                    # Brightness keeps the print; 0.3 floor so dark prints still glow.
                    $l = (0.3 + 0.7 * (0.299 * $px[$i + 2] + 0.587 * $px[$i + 1] + 0.114 * $px[$i]) / 255.0)
                    $buf[$i] = [byte][Math]::Min(255, $rgb[2] * $l); $buf[$i + 1] = [byte][Math]::Min(255, $rgb[1] * $l)
                    $buf[$i + 2] = [byte][Math]::Min(255, $rgb[0] * $l); $buf[$i + 3] = $px[$i + 3]
                }
                $tex.Format = [CodeWalker.GameFiles.TextureFormat]::D3DFMT_A8R8G8B8
                $tex.Levels = 1
                $tex.Stride = [uint16]($tex.Width * 4)
                $tex.Data.FullData = $buf
                [IO.File]::WriteAllBytes($dds, [CodeWalker.Utils.DDSIO]::GetDDSFile($tex))
                $item.Format = 'D3DFMT_A8R8G8B8'
                $item.MipLevels.SetAttribute('value', '1')
            }
            Write-Resource $vdoc "$variant.ydr" $tinted (Join-Path $Out "$variant.ydr")
            $vxml = $typ.OuterXml.Replace(">$name<", ">$variant<").Replace(">$hash<", ">$variant<")
            Write-Resource ([xml]$vxml) "$variant.ytyp" (Join-Path $work 'ytyp') (Join-Path $Out "$variant.ytyp")
        }
    }
    Remove-Item -LiteralPath $work -Recurse -Force
    $done++
}
"$done glowing wheel copies in $Out"
if ($Install) {
    if (Get-Process GTA5 -ErrorAction SilentlyContinue) { throw 'Close GTA first.' }
    Copy-Item -Path (Join-Path $Out '*_glow*.y*') -Destination $Parts -Force
    "installed into $Parts"
}
