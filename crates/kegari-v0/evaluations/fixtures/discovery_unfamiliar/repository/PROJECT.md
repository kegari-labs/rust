# Current project contract

The limit is inclusive. This repository uses its own small development tool,
`dev/reef`; inspect its help and source to derive the supported validation call.
It accepts `check RESULT`, runs the ordinary boundary tests, writes the structured
`boundary` result to RESULT and returns the actual test status. Put RESULT in the
session scratch directory. Do not install a tool, replace this one or add Kegari
command configuration. Only the assigned source path may change.
