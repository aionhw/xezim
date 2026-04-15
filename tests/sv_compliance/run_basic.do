source [file join [file dirname [file normalize [info script]]] questa_common.do]
set run_time 20us
if {[info exists ::env(RUN_TIME)]} {
    set run_time $::env(RUN_TIME)
}
set rc [qsuite_run_group basic [qsuite_get_files basic] positive $run_time]
qsuite_finish $rc
