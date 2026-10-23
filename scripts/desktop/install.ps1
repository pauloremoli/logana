  # Use the embedded executable icon for both Windows shortcuts.
  $shell = New-Object -ComObject WScript.Shell
  foreach ($folder in @([Environment]::GetFolderPath('Desktop'), [Environment]::GetFolderPath('Programs'))) {
    if ($folder) {
      $shortcut = $shell.CreateShortcut((Join-Path $folder 'Logana.lnk'))
      $shortcut.TargetPath = Join-Path $dest_dir 'logana-gui.exe'
      $shortcut.WorkingDirectory = $dest_dir
      $shortcut.IconLocation = "$($shortcut.TargetPath),0"
      $shortcut.Save()
    }
  }
