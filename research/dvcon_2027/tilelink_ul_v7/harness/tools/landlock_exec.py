#!/usr/bin/env python3
"""Run a command with Landlock restricting file reads to explicit paths (Linux)."""
import argparse, ctypes, os, sys
libc=ctypes.CDLL(None,use_errno=True)
CREATE,ADD,RESTRICT=444,445,446
EXECUTE,WRITE,READ_FILE,READ_DIR=1,2,4,8
class Ruleset(ctypes.Structure): _fields_=[('handled_access_fs',ctypes.c_uint64)]
class PathRule(ctypes.Structure): _fields_=[('allowed_access',ctypes.c_uint64),('parent_fd',ctypes.c_int32)]
def call(n,*args):
 v=libc.syscall(n,*args)
 if v<0: raise OSError(ctypes.get_errno(),os.strerror(ctypes.get_errno()))
 return v
def restrict(paths):
 ruleset=Ruleset(EXECUTE|READ_FILE|READ_DIR)
 fd=call(CREATE,ctypes.byref(ruleset),ctypes.sizeof(ruleset),0)
 for path in paths:
  p=os.path.realpath(path)
  if not os.path.exists(p): continue
  pfd=os.open(p,os.O_PATH|os.O_CLOEXEC)
  access=EXECUTE|READ_FILE|(READ_DIR if os.path.isdir(p) else 0)
  rule=PathRule(access,pfd)
  try: call(ADD,fd,1,ctypes.byref(rule),0)
  finally: os.close(pfd)
 if libc.prctl(38,1,0,0,0): raise OSError(ctypes.get_errno(),os.strerror(ctypes.get_errno()))
 call(RESTRICT,fd,0);os.close(fd)
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--allow',action='append',default=[]);p.add_argument('command',nargs=argparse.REMAINDER);a=p.parse_args();cmd=a.command
 if cmd and cmd[0]=='--':cmd=cmd[1:]
 if not cmd: p.error('command required')
 restrict(a.allow);os.execvp(cmd[0],cmd)
