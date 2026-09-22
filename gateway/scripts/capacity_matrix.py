#!/usr/bin/env python3
"""Run final HTTP scenarios sequentially; every sustained load lasts 60 seconds."""
import argparse
import json
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument('--binary', required=True)
parser.add_argument('--output-dir', required=True)
args = parser.parse_args()
output = Path(args.output_dir)
output.mkdir(parents=True, exist_ok=True)
script = Path(__file__).with_name('capacity.py')
cases = [(mode, trace, clients, 478000, 60 if clients else 1, 1)
         for clients in (50, 0, 32, 128)
         for mode in ('native', 'transform')
         for trace in ('on', 'off')]
cases += [('ir', 'on', 50, 478000, 60, 1),
          ('iterate', 'on', 50, 800000, 60, 1)]
cases += [('ir', 'on', 50, 8800, 5, threads) for threads in (1, 2, 4)]
completed = []
for mode, trace, clients, size, seconds, threads in cases:
    name = f'{mode}-{trace}-{clients}-{size}-{threads}'
    command = ['python3', str(script), '--binary', args.binary,
               '--mode', mode, '--trace', trace, '--clients', str(clients),
               '--body-bytes', str(size), '--seconds', str(seconds),
               '--threads', str(threads), '--output', str(output / (name + '.json'))]
    with (output / (name + '.log')).open('w') as log:
        subprocess.run(command, stdout=log, stderr=subprocess.STDOUT,
                       timeout=180, check=True)
    completed.append(name)
    (output / 'completed.json').write_text(json.dumps(completed, indent=2) + '\n')
    print(name, flush=True)
