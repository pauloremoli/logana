- support ingesting all kinds of common log formats from linux to webserver patterns (rust, python, node common loggers included) and structured logging

- for structured logging a view could be created grouping based on some specific field (e.g by request_id, user_id) or for some specific value for that field (only request_id=10)

- command to open a view in a new tab with only marked lines

- command to select the fields to show (and also order)

- visual mode like in neovim (to allow copying into clipboard, or add filter/search based on selected text)
  - with complete set for vim motions

- go to line number (or closest line if hidden by filter)

- special filter for date range based on timestamp, if a timestamp field was not identified when opening the file ask the user to identify the column

- support for commentary/annotation, those could be appplied for a group of lines so there should be a way to select multiple lines
  - the idea is to later export a markdown file (or Jira format) with all commentaries and marked lines, the commentary should be above the marked lines 
  define a template, allow the user to edit the template (put the the template in the installation folder next to the themes)
[[Log file]] 
File name: <FILENAME> 

[[Analysis]]

  <COMMENTARY>
  ```<MARKED_LINES>```
...

```more MARKED_LINES```

- command to preview analysis export

- make all shortcuts configurable

- support for opening a dir with log files, each in separate tab, if there is a timestamp field ask the user if it would like a view with the merged timeline from all logs sorted by the timestamp
