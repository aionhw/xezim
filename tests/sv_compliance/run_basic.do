source [file join [file dirname [file normalize [info script]]] xxxyyy_common.script]
set run_time 20us
if {[info exists ::env(RUN_TIME)]} {
    set run_time $::env(RUN_TIME)
}
set rc [qsuite_run_group basic [qsuite_get_files basic] positive $run_time]
qsuite_finish $rc
