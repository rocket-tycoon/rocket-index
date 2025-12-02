; Bracket pairs for F# in Zed

; Standard brackets
("(" @open ")" @close)
("[" @open "]" @close)
("{" @open "}" @close)

; F#-specific array brackets
("[|" @open "|]" @close)

; Attribute brackets
("[<" @open ">]" @close)

; Begin/end blocks
("begin" @open "end" @close)
