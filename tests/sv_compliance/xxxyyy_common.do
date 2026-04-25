# Common Tcl helpers for the SystemVerilog compliance suite on xxxyyy/ModelSim.

set ::QSUITE_DO_DIR [file dirname [file normalize [info script]]]
set ::QSUITE_ROOT   [file dirname $::QSUITE_DO_DIR]
set ::QSUITE_COMMON [file normalize [file join $::QSUITE_ROOT common]]
set ::QSUITE_SIM    [file normalize [file join $::QSUITE_ROOT sim]]
set ::QSUITE_LIBS   [file normalize [file join $::QSUITE_SIM libs]]
set ::QSUITE_LOGS   [file normalize [file join $::QSUITE_SIM logs]]

proc qsuite_banner {msg} {
    puts "\n=== $msg ==="
}

proc qsuite_note {msg} {
    puts $msg
}

proc qsuite_warn {msg} {
    puts stderr $msg
}

proc qsuite_prepare_dirs {} {
    foreach dir [list $::QSUITE_SIM $::QSUITE_LIBS $::QSUITE_LOGS] {
        if {![file exists $dir]} {
            file mkdir $dir
        }
    }
}

proc qsuite_read_file {path} {
    set fh [open $path r]
    set data [read $fh]
    close $fh
    return $data
}

proc qsuite_write_file {path data} {
    set fh [open $path w]
    puts -nonewline $fh $data
    close $fh
}

proc qsuite_append_file {path data} {
    set fh [open $path a]
    puts -nonewline $fh $data
    close $fh
}

proc qsuite_lib_name_from_file {file_path} {
    set stem [file rootname [file tail $file_path]]
    return [string map {"-" "_" "." "_"} "lib_${stem}"]
}

proc qsuite_positive_top_from_file {file_path} {
    set stem [file rootname [file tail $file_path]]
    regsub {^[0-9]+_} $stem {} stem
    return "test_${stem}"
}

proc qsuite_negative_top_from_file {file_path} {
    return [file rootname [file tail $file_path]]
}

proc qsuite_reset_library {file_path} {
    qsuite_prepare_dirs

    set lib_name [qsuite_lib_name_from_file $file_path]
    set lib_dir  [file join $::QSUITE_LIBS $lib_name]

    if {[file exists $lib_dir]} {
        file delete -force $lib_dir
    }

    catch {vmap -del $lib_name}
    vlib $lib_dir
    vmap $lib_name $lib_dir
    return $lib_name
}

proc qsuite_get_files {kind} {
    switch -- $kind {
        basic {
            return [lsort [glob -nocomplain -directory [file join $::QSUITE_ROOT tests] *.sv]]
        }
        advanced {
            return [lsort [glob -nocomplain -directory [file join $::QSUITE_ROOT tests_advanced] *.sv]]
        }
        positive {
            return [concat [qsuite_get_files basic] [qsuite_get_files advanced]]
        }
        negative {
            return [lsort [glob -nocomplain -directory [file join $::QSUITE_ROOT tests_negative] *.sv]]
        }
        default {
            error "Unknown file group: $kind"
        }
    }
}

proc qsuite_list_tests {} {
    foreach group {basic advanced negative} {
        qsuite_banner "${group} tests"
        foreach f [qsuite_get_files $group] {
            puts [file tail $f]
        }
    }
}

proc qsuite_resolve_test {name} {
    if {[file exists $name]} {
        return [file normalize $name]
    }

    foreach dir [list tests tests_advanced tests_negative] {
        set candidate [file join $::QSUITE_ROOT $dir $name]
        if {[file exists $candidate]} {
            return [file normalize $candidate]
        }
    }

    if {![string match "*.sv" $name]} {
        foreach dir [list tests tests_advanced tests_negative] {
            foreach candidate [glob -nocomplain -directory [file join $::QSUITE_ROOT $dir] ${name}.sv] {
                return [file normalize $candidate]
            }
        }
    }

    error "Could not locate test file: $name"
}

proc qsuite_is_negative_test {file_path} {
    return [string match "neg*.sv" [file tail $file_path]]
}

proc qsuite_run_positive_test {file_path {run_time 20us}} {
    qsuite_prepare_dirs

    set file_path [file normalize $file_path]
    set file_name [file tail $file_path]
    set test_name [file rootname $file_name]
    set top_name  [qsuite_positive_top_from_file $file_path]
    set lib_name  [qsuite_reset_library $file_path]
    set log_dir   [file join $::QSUITE_LOGS $test_name]
    set compile_log [file join $log_dir compile.log]
    set run_log     [file join $log_dir run.log]

    if {[file exists $log_dir]} {
        file delete -force $log_dir
    }
    file mkdir $log_dir

    qsuite_banner "Running $file_name (top=$top_name, time=$run_time)"

    set COMPILE_cmd [list COMPILE -sv -l $compile_log -work $lib_name "+incdir+$::QSUITE_COMMON" $file_path]
    if {[catch {eval $COMPILE_cmd} msg]} {
        qsuite_warn "[FAIL] compile failed for $file_name"
        qsuite_warn "       see $compile_log"
        return 1
    }

    set SIM_cmd [list SIM -c -onfinish stop -l $run_log ${lib_name}.${top_name}]
    if {[catch {eval $SIM_cmd} msg]} {
        qsuite_warn "[FAIL] elaboration failed for $file_name"
        qsuite_warn "       see $run_log"
        return 1
    }

    set run_rc [catch {
        run $run_time
    } run_msg]

    catch {quit -sim}

    if {$run_rc} {
        qsuite_warn "[FAIL] runtime error for $file_name"
        qsuite_warn "       see $run_log"
        return 1
    }

    set log_data [qsuite_read_file $run_log]
    if {[string first "TEST_PASS" $log_data] >= 0} {
        qsuite_note "[PASS] $file_name"
        return 0
    }

    qsuite_warn "[FAIL] TEST_PASS not found for $file_name"
    qsuite_warn "       see $run_log"
    return 1
}

proc qsuite_run_negative_test {file_path} {
    qsuite_prepare_dirs

    set file_path [file normalize $file_path]
    set file_name [file tail $file_path]
    set test_name [file rootname $file_name]
    set lib_name  [qsuite_reset_library $file_path]
    set log_dir   [file join $::QSUITE_LOGS $test_name]
    set compile_log [file join $log_dir compile.log]

    if {[file exists $log_dir]} {
        file delete -force $log_dir
    }
    file mkdir $log_dir

    qsuite_banner "Compiling negative test $file_name (expect fail)"

    set COMPILE_cmd [list COMPILE -sv -l $compile_log -work $lib_name "+incdir+$::QSUITE_COMMON" $file_path]
    if {[catch {eval $COMPILE_cmd} msg]} {
        qsuite_note "[PASS] $file_name compile failed as expected"
        return 0
    }

    qsuite_warn "[FAIL] $file_name compiled successfully but should fail"
    qsuite_warn "       see $compile_log"
    return 1
}

proc qsuite_emit_summary {label passed failed} {
    qsuite_prepare_dirs
    set summary_file [file join $::QSUITE_SIM summary_${label}.txt]
    set out "Regression: $label\nPassed: [llength $passed]\nFailed: [llength $failed]\n\n"

    if {[llength $passed] > 0} {
        append out "PASS\n"
        foreach item $passed {
            append out "  $item\n"
        }
        append out "\n"
    }

    if {[llength $failed] > 0} {
        append out "FAIL\n"
        foreach item $failed {
            append out "  $item\n"
        }
        append out "\n"
    }

    qsuite_write_file $summary_file $out
    qsuite_note "Summary: $summary_file"
}

proc qsuite_run_group {label files mode {run_time 20us}} {
    if {[llength $files] == 0} {
        qsuite_warn "No tests found for group: $label"
        return 1
    }

    set passed {}
    set failed {}

    foreach f $files {
        if {$mode eq "positive"} {
            set rc [qsuite_run_positive_test $f $run_time]
        } elseif {$mode eq "negative"} {
            set rc [qsuite_run_negative_test $f]
        } else {
            error "Unknown run mode: $mode"
        }

        if {$rc == 0} {
            lappend passed [file tail $f]
        } else {
            lappend failed [file tail $f]
        }
    }

    qsuite_emit_summary $label $passed $failed

    if {[llength $failed] == 0} {
        qsuite_note "\n[DONE] $label passed ([llength $passed] tests)"
        return 0
    }

    qsuite_warn "\n[FAIL] $label had [llength $failed] failing tests"
    return 1
}

proc qsuite_finish {rc} {
    if {$rc == 0} {
        quit -f -code 0
    } else {
        quit -f -code 1
    }
}
