"""Bounded public-pin VCD reader. End-of-time-slot waveform, not native evidence."""
PINS={'a_valid','a_ready','a_opcode','a_size','a_source','a_address','a_mask','a_data','d_valid','d_ready','d_opcode','d_size','d_source','d_data','d_error','rst_n','request_throttle','monitor_alarm','scoreboard_alarm'}
def read(path):
 codes={};scopes=[];tick=0;changes={};rows=[]
 def flush():
  nonlocal changes
  if changes:rows.append({'time_ticks':tick,'phase':'vcd_end_of_time_slot','changes':changes});changes={}
 for line in path.read_text().splitlines():
  parts=line.split()
  if line.startswith('$scope'):scopes.append(parts[2])
  elif line.startswith('$upscope'):scopes.pop()
  elif line.startswith('$var') and scopes==['tb_tlul'] and parts[4] in PINS:codes.setdefault(parts[3],[]).append((parts[4],int(parts[2])))
  elif line.startswith('#'):flush();tick=int(line[1:])
  elif parts and line[0].lower() in 'b01xz':
   if line[0].lower()=='b' and len(parts)==2:code=parts[1];value=parts[0][1:].lower()
   else:code=line[1:];value=line[0].lower()
   for name,width in codes.get(code,[]):changes[name]=value.rjust(width,value[0] if value[0] in 'xz' else '0')[-width:]
 flush()
 if not codes or not rows:raise ValueError('no public-pin VCD evidence')
 return rows
