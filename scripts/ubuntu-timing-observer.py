#!/usr/bin/env python3
"""One-off host chronology. An instrumented run is not budget admission."""

import json
import os
from pathlib import Path
import re
import signal
import stat
import subprocess
import sys
import tempfile
import time
import uuid


TRACEFS = Path('/sys/kernel/tracing')
EVENTS = ('fork', 'exec', 'exit')
LOSS_FIELDS = ('overrun', 'commit overrun', 'dropped events')
RAW_LIMIT = 7 * 1024 * 1024
PUBLIC_LIMIT = 8 * 1024 * 1024
REFUSAL_STAGES = frozenset((
    'observer-check', 'observer-setup', 'control-user', 'control-name', 'control-instance',
    'tracefs-mounted', 'create-instance', 'create-stop', 'create-clock', 'create-buffer',
    'config', 'config-tracer', 'config-clock', 'config-stacks', 'config-events',
    'config-filters', 'config-triggers', 'config-fork', 'config-exec', 'config-exit',
    'config-pids', 'config-buffer', 'runner-user', 'runner-temp', 'runner-private',
))
EXECUTABLES = frozenset((
    'pinfold', 'firecracker', 'mkfs.ext4', 'mke2fs', 'cargo', 'rustc',
    'cc', 'gcc', 'cc1', 'collect2', 'ld', 'as', 'musl-gcc', 'git',
    'sh', 'dash', 'bash', 'curl', 'openssl', 'python3', 'sudo', 'true',
    'cat', 'supervise',
))
LINE = re.compile(
    r'^[^\r\n]{1,16}-\d+\s+\[\d+\]\s+\S+\s+(\d+)\.(\d+): '
    r'sched_process_(fork|exec|exit): (.*)$'
)
PAYLOAD = {
    'fork': re.compile(r'comm=(.*?) pid=(\d+) child_comm=(.*?) child_pid=(\d+)$'),
    'exec': re.compile(r'filename=(.*?) pid=(\d+) old_pid=(\d+)$'),
    'exit': re.compile(r'comm=(.*?) pid=(\d+) prio=\d+(?: group_dead=(?:true|false))?$'),
}


class Refusal(ValueError):
    def __init__(self, stage, errno=None):
        self.stage = stage if stage in REFUSAL_STAGES else 'observer-check'
        self.errno = errno if type(errno) is int and 0 < errno < 4096 else None


def refusal(error, stage='observer-check'):
    result = {'refusal_stage': error.stage if isinstance(error, Refusal) else stage}
    if result['refusal_stage'] not in REFUSAL_STAGES:
        result['refusal_stage'] = 'observer-check'
    if isinstance(error, (Refusal, OSError)) and type(error.errno) is int and 0 < error.errno < 4096:
        result['errno'] = error.errno
    return result


def require(condition, stage='observer-check'):
    if not condition:
        raise Refusal(stage)


def checked_read(path, stage):
    try:
        return path.read_text()
    except OSError as error:
        raise Refusal(stage, error.errno) from None


def private_file(path):
    return os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'wb')


def stats(instance):
    result = {}
    for path in sorted((instance / 'per_cpu').glob('cpu[0-9]*/stats')):
        fields = dict(line.split(':', 1) for line in path.read_text().splitlines())
        result[int(path.parent.name[3:])] = {
            key: int(fields[key].strip())
            for key in ('entries', *LOSS_FIELDS)
        }
    require(bool(result))
    return result


def configuration(instance, enabled):
    require(checked_read(instance / 'current_tracer', 'config-tracer').strip() == 'nop', 'config-tracer')
    require('[mono]' in checked_read(instance / 'trace_clock', 'config-clock').split(), 'config-clock')
    options = set(checked_read(instance / 'trace_options', 'config-stacks').split())
    require({'nostacktrace', 'nouserstacktrace'} <= options, 'config-stacks')
    require(not {
        'stacktrace', 'userstacktrace', 'func_stack_trace', 'branch', 'event-fork', 'function-fork',
    } & options, 'config-stacks')
    selected = {f'sched_process_{event}' for event in EVENTS}
    for path in (instance / 'events').glob('*/*/enable'):
        wanted = path.parent.parent.name == 'sched' and path.parent.name in selected
        require(checked_read(path, 'config-events').strip() == ('1' if wanted and enabled else '0'), 'config-events')
        require(checked_read(path.parent / 'filter', 'config-filters').strip() in ('', 'none', '0'), 'config-filters')
        require(not any(
            line.strip() and not line.lstrip().startswith('#')
            for line in checked_read(path.parent / 'trigger', 'config-triggers').splitlines()
        ), 'config-triggers')
    schemas = {}
    fixed_fork = {'char parent_comm[16]': (8, 16), 'pid_t parent_pid': (24, 4),
                  'char child_comm[16]': (28, 16), 'pid_t child_pid': (44, 4)}
    string_fork = {'__data_loc char[] parent_comm': (8, 4), 'pid_t parent_pid': (12, 4),
                   '__data_loc char[] child_comm': (16, 4), 'pid_t child_pid': (20, 4)}
    fixed_exit = {'char comm[16]': (8, 16), 'pid_t pid': (24, 4), 'int prio': (28, 4)}
    layouts = {
        'fork': (
            ('comm=%s pid=%d child_comm=%s child_pid=%d', fixed_fork),
            ('comm=%s pid=%d child_comm=%s child_pid=%d', string_fork),
        ),
        'exec': (('filename=%s pid=%d old_pid=%d', {
            '__data_loc char[] filename': (8, 4), 'pid_t pid': (12, 4), 'pid_t old_pid': (16, 4),
        }),),
        'exit': (
            ('comm=%s pid=%d prio=%d', fixed_exit),
            ('comm=%s pid=%d prio=%d group_dead=%s', {**fixed_exit, 'bool group_dead': (32, 1)}),
        ),
    }
    for event, variants in layouts.items():
        stage = f'config-{event}'
        text = checked_read(instance / 'events/sched' / f'sched_process_{event}/format', stage)
        declarations = [
            (declaration, offset, size)
            for declaration, offset, size in re.findall(
                r'field:([^;]+);\s*offset:(\d+);\s*size:(\d+);', text
            ) if not declaration.split()[-1].startswith('common_')
        ]
        fields = {
            ' '.join(declaration.split()): (int(offset), int(size))
            for declaration, offset, size in declarations
        }
        require(len(fields) == len(declarations), stage)
        require(any(
            fields == expected and f'print fmt: "{layout}"' in text
            for layout, expected in variants
        ), stage)
        schemas[event] = {
            declaration.rsplit(' ', 1)[-1].split('[', 1)[0]: {'offset': offset, 'size': size}
            for declaration, (offset, size) in fields.items()
        }
    require(checked_read(instance / 'set_event_pid', 'config-pids').strip() == '', 'config-pids')
    require(checked_read(instance / 'set_event_notrace_pid', 'config-pids').strip() == '', 'config-pids')
    total_kib = int(checked_read(instance / 'buffer_total_size_kb', 'config-buffer'))
    require(0 < total_kib <= 4096, 'config-buffer')
    return {'nop': True, 'mono': True, 'only_three_events': True,
            'no_stacks_triggers_filters': True, 'buffer_total_kib': total_kib,
            'formats': schemas}


def remove(instance):
    result = {}
    for key, action in (
        ('stopped', lambda: (instance / 'tracing_on').write_text('0')),
        ('events_disabled', lambda: (instance / 'events/enable').write_text('0')),
        ('removed', instance.rmdir),
    ):
        try:
            action()
            result[key] = not instance.exists() if key == 'removed' else True
        except BaseException:
            result[key] = False
    return result


def control(operation, name):
    require(os.geteuid() == 0, 'control-user')
    require(re.fullmatch(r'pinfold-timing-\d+-[a-f0-9]{12}', name) is not None, 'control-name')
    instance = TRACEFS / 'instances' / name
    if operation == 'create':
        created = False
        stage = 'tracefs-mounted'
        try:
            mounted = any(
                line.split(' - ', 1)[0].split()[4] == str(TRACEFS)
                and line.split(' - ', 1)[1].split()[0] == 'tracefs'
                for line in checked_read(Path('/proc/self/mountinfo'), stage).splitlines()
            )
            require(mounted, stage)
            stage = 'create-instance'
            instance.mkdir()
            created = True
            stage = 'create-stop'
            (instance / 'tracing_on').write_text('0')
            stage = 'create-clock'
            (instance / 'trace_clock').write_text('mono')
            stage = 'create-buffer'
            (instance / 'buffer_size_kb').write_text('1024')
            stage = 'config'
            return {'ok': True, 'created': True, 'configuration': configuration(instance, False)}
        except BaseException as error:
            result = {'ok': False, 'created': created, **refusal(error, stage)}
            if created:
                result['cleanup'] = remove(instance)
            return result
    require(instance.is_dir(), 'control-instance')
    if operation == 'start':
        require((instance / 'tracing_on').read_text().strip() == '0')
        (instance / 'trace').write_text('')
        for event in EVENTS:
            (instance / 'events/sched' / f'sched_process_{event}/enable').write_text('1')
        checked = configuration(instance, True)
        before = stats(instance)
        (instance / 'tracing_on').write_text('1')
        require((instance / 'tracing_on').read_text().strip() == '1')
        return {'ok': True, 'configuration': checked, 'stats': before}
    if operation == 'stop':
        (instance / 'tracing_on').write_text('0')
        require((instance / 'tracing_on').read_text().strip() == '0')
        return {'ok': True, 'stopped': True}
    if operation == 'stats':
        return {'ok': True, 'stats': stats(instance)}
    if operation == 'read':
        require((instance / 'tracing_on').read_text().strip() == '0')
        # Unfiltered host metadata must never go to a CI pipe or terminal.
        destination = os.fstat(sys.stdout.fileno())
        require(stat.S_ISREG(destination.st_mode))
        require(stat.S_IMODE(destination.st_mode) == 0o600 and destination.st_nlink == 1)
        require(destination.st_uid == int(os.environ['SUDO_UID']))
        with (instance / 'trace').open('rb') as source:
            remaining = RAW_LIMIT + 1
            while remaining:
                chunk = source.read(min(65536, remaining))
                if not chunk:
                    break
                sys.stdout.buffer.write(chunk)
                remaining -= len(chunk)
        return None
    if operation == 'remove':
        return {'ok': True, **remove(instance)}
    raise ValueError('unknown observer operation')


def test_names():
    names = {}
    for module, filename in (
        ('box_', 'box_'), ('cli', 'cli'), ('cli::update', 'update'),
        ('pi', 'pi'), ('runs', 'runs'), ('runtime', 'runtime'),
    ):
        text = Path(f'crates/e2e/tests/e2e/{filename}.rs').read_text()
        for name in re.findall(r'#\[test\]\s*(?:#\[[^\]]+\]\s*)*fn ([a-z][a-z0-9_]*)\(', text):
            public = f'{module}::{name}'
            names.setdefault(public[:15], []).append(public)
    return names


def header_lines(raw):
    require(raw.endswith(b'\n'))
    lines = raw.split(b'\n')[:-1]
    require(bool(lines) and all(not line or line.startswith(b'#') for line in lines))
    return normalized_header(lines)


def normalized_header(lines):
    count_line = re.compile(rb'(# entries-in-buffer/entries-written:) \d+/\d+( +#P:)\d+')
    found = 0
    result = []
    for line in lines:
        if count_line.fullmatch(line):
            line = count_line.sub(rb'\1 <counts>\2<cpus>', line)
            found += 1
        result.append(line)
    require(found == 1)
    return result


def framed_lines(raw, header, final_stats):
    require(raw.endswith(b'\n'))
    lines = raw.split(b'\n')[:-1]
    require(normalized_header(lines[:len(header)]) == header)
    body = lines[len(header):]
    # The validated print formats add one LF per native event. Counting every
    # body line against separate kernel counters rejects embedded-LF records.
    require(len(body) == sum(cpu['entries'] for cpu in final_stats.values()))
    require(all(body_line and not body_line.startswith(b'#') for body_line in body))
    return body


def sanitize(lines, wrapper, names, destination):
    active = {wrapper: 0}
    next_task = 1
    ambiguous = set()
    result = {'parser_gaps': 0, 'lost_markers': 0, 'retention_overflow': False,
              'unknown_ancestry': 0, 'ambiguous_identity': 0, 'events': 0}
    readiness = {}
    used = 0
    for body_line in lines:
        try:
            line = body_line.decode('utf-8')
        except UnicodeDecodeError:
            result['parser_gaps'] += 1
            continue
        if not line.isprintable():
            result['parser_gaps'] += 1
            continue
        if re.fullmatch(r'\s*CPU:\d+ \[LOST \d+ EVENTS\]\s*', line):
            result['lost_markers'] += 1
            continue
        match = LINE.fullmatch(line)
        if not match:
            result['parser_gaps'] += 1
            continue
        seconds, fraction, event, payload = match.groups()
        parsed = PAYLOAD[event].fullmatch(payload)
        if not parsed or len(fraction) > 9:
            result['parser_gaps'] += 1
            continue
        stamp = int(seconds) * 1_000_000_000 + int(fraction.ljust(9, '0'))
        record = {'event': event, 'monotonic_ns': stamp, 'attribution': 'AMBIGUOUS'}
        comm = None
        if event == 'fork':
            comm, parent, _child_comm, pid = parsed.groups()
            parent, pid = int(parent), int(pid)
            if pid in active:
                ambiguous.add(pid)
                active.pop(pid)
                result['ambiguous_identity'] += 1
            if parent not in active or parent in ambiguous or pid in ambiguous:
                result['unknown_ancestry'] += 1
                continue
            task = next_task
            next_task += 1
            active[pid] = task
            record.update(pid=pid, task=task, parent_pid=parent, parent_task=active[parent])
            if parent == wrapper:
                readiness[(pid, task)] = {'fork'}
        elif event == 'exec':
            filename, pid, old_pid = parsed.groups()
            pid, old_pid = int(pid), int(old_pid)
            if old_pid != pid and (pid in active or old_pid in active):
                result['ambiguous_identity'] += 1
                for identity in (pid, old_pid):
                    active.pop(identity, None)
                    ambiguous.add(identity)
                continue
            if pid not in active or pid in ambiguous:
                result['unknown_ancestry'] += 1
                continue
            basename = filename.rsplit('/', 1)[-1]
            category = basename if basename in EXECUTABLES else 'redacted'
            record.update(pid=pid, task=active[pid], old_pid=old_pid, executable=category)
            if filename == '/usr/bin/true' and (pid, active[pid]) in readiness:
                readiness[(pid, active[pid])].add('exec')
        else:
            comm, pid = parsed.groups()
            pid = int(pid)
            if pid not in active or pid in ambiguous:
                result['unknown_ancestry'] += 1
                ambiguous.discard(pid)
                continue
            task = active.pop(pid)
            record.update(pid=pid, task=task)
            if (pid, task) in readiness:
                readiness[(pid, task)].add('exit')
        candidates = names.get(comm, [])
        key = 'parent_test_candidate' if event == 'fork' else 'test_candidate'
        record[key] = candidates[0] if len(candidates) == 1 else 'redacted'
        encoded = (json.dumps(record, separators=(',', ':')) + '\n').encode()
        if used + len(encoded) > PUBLIC_LIMIT - 65536:
            result['retention_overflow'] = True
            continue
        destination.write(encoded)
        used += len(encoded)
        result['events'] += 1
    result['retained_bytes'] = used
    return result, readiness


def no_loss(before, after):
    return bool(before) and before.keys() == after.keys() and all(
        after[cpu][key] == before[cpu][key] == 0
        for cpu in before for key in LOSS_FIELDS
    )


def parsed_clean(parsed):
    return not any(parsed[key] for key in (
        'parser_gaps', 'lost_markers', 'retention_overflow', 'ambiguous_identity'
    ))


def main():
    require(os.geteuid() != 0, 'runner-user')
    temporary = Path(os.environ['RUNNER_TEMP'])
    require(temporary.is_absolute() and temporary.is_dir(), 'runner-temp')
    directory = Path(tempfile.mkdtemp(prefix='pinfold-timing-', dir=temporary))
    require(stat.S_IMODE(directory.stat().st_mode) == 0o700, 'runner-private')
    name = f'pinfold-timing-{os.getpid()}-{uuid.uuid4().hex[:12]}'
    receipt = {'diagnostic_only': True, 'wrapper_pid': os.getpid(), 'suite_started': False,
               'suite_exit': None, 'capture_complete': False, 'attribution_complete': False,
               'hard_kill_has_no_cleanup_receipt': True, 'cleanup': {}}
    created = False
    child = cancelled = None
    timeline = directory / 'sanitized.jsonl'
    names = test_names()

    def call(operation, output=None):
        completed = subprocess.run(
            ['sudo', '-n', sys.executable, str(Path(__file__).resolve()), '--control', operation, name],
            stdin=subprocess.DEVNULL,
            stdout=output if output is not None else subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        require(completed.returncode == 0)
        if output is not None:
            return None
        result = json.loads(completed.stdout)
        # A failed create still acknowledges ownership for independent cleanup.
        if operation != 'create':
            require(result.get('ok') is True)
        return result

    def read_trace(label):
        path = directory / f'{label}.trace'
        with private_file(path) as output:
            call('read', output)
        raw = path.read_bytes()
        return raw[:RAW_LIMIT], len(raw) > RAW_LIMIT

    def cancelled_by(signum, _frame):
        nonlocal cancelled
        cancelled = signum
        if child is not None and child.poll() is None:
            try:
                child.send_signal(signum)
            except ProcessLookupError:
                pass

    for signum in (signal.SIGINT, signal.SIGTERM):
        signal.signal(signum, cancelled_by)
    try:
        receipt['preflight'] = call('create')
        created = receipt['preflight'].get('created', False) and not receipt['preflight'].get(
            'cleanup', {}
        ).get('removed', False)
        require(receipt['preflight']['ok'] and not cancelled)
        empty, overflow = read_trace('empty')
        require(not overflow)
        header = header_lines(empty)
        empty_stats = call('stats')['stats']
        require(all(cpu['entries'] == 0 for cpu in empty_stats.values()))
        receipt['empty_header'] = {'lines': len(header), 'bytes': len(empty), 'entries': 0}
        before = call('start')
        require(before['ok'] and not cancelled)
        child = subprocess.Popen(['/usr/bin/true'])
        if cancelled:
            child.send_signal(cancelled)
        probe_pid, probe_code = child.pid, child.wait()
        child = None
        require(call('stop')['ok'])
        after = call('stats')
        raw, overflow = read_trace('readiness')
        receipt['readiness'] = {'passed': False, 'child_pid': probe_pid,
                                'raw_overflow': overflow, 'before': before, 'after': after}
        body = framed_lines(raw, header, after['stats'])
        with private_file(directory / 'readiness.jsonl') as output:
            parsed, seen = sanitize(body, os.getpid(), names, output)
        lifetimes = [events for (pid, _task), events in seen.items() if pid == probe_pid]
        ready = probe_code == 0 and lifetimes == [{'fork', 'exec', 'exit'}]
        ready = ready and not overflow and parsed_clean(parsed) and no_loss(before['stats'], after['stats'])
        receipt['readiness'].update(passed=ready, framing_validated=True, parser=parsed)
        require(ready and not cancelled)
        receipt['baseline'] = call('start')
        require(receipt['baseline']['ok'] and not cancelled)
        receipt['suite_start_monotonic_ns'] = time.monotonic_ns()
        child = subprocess.Popen(['python3', 'scripts/e2e-budget.py'])
        receipt.update(suite_started=True, suite_pid=child.pid)
        if cancelled:
            child.send_signal(cancelled)
        receipt['suite_exit'] = child.wait()
        receipt['suite_end_monotonic_ns'] = time.monotonic_ns()
    except BaseException as error:
        receipt['observer_error'] = True
        receipt.update(refusal(error))
    finally:
        if child is not None:
            code = child.wait()
            if receipt['suite_started']:
                receipt['suite_exit'] = code
                receipt['suite_end_monotonic_ns'] = time.monotonic_ns()
        if created:
            stopped = False
            try:
                receipt['cleanup']['stop'] = call('stop')
                stopped = receipt['cleanup']['stop']['ok']
            except BaseException:
                receipt['cleanup']['stop'] = False
            try:
                receipt['final'] = call('stats')
            except BaseException:
                receipt['cleanup']['stats'] = False
            if stopped and receipt['suite_started']:
                try:
                    raw, overflow = read_trace('suite')
                    receipt['raw_overflow'] = overflow
                    body = framed_lines(raw, header, receipt['final']['stats'])
                    with private_file(timeline) as output:
                        parsed, _ = sanitize(body, os.getpid(), names, output)
                    receipt.update(parser=parsed, framing_validated=True)
                    receipt['capture_complete'] = not overflow and parsed_clean(parsed) and no_loss(
                        receipt['baseline']['stats'], receipt['final']['stats']
                    )
                except BaseException:
                    receipt['cleanup']['read'] = False
                    try:
                        timeline.unlink(missing_ok=True)
                    except OSError:
                        pass
            try:
                receipt['cleanup']['remove'] = call('remove')
                receipt['capture_complete'] &= all(receipt['cleanup']['remove'].values())
            except BaseException:
                receipt['cleanup']['remove'] = False
                receipt['capture_complete'] = False
        receipt['cancelled_signal'] = cancelled
        if cancelled:
            # Preserve the budget wrapper's existing lifecycle. The CI runner
            # owns job cancellation; this observer cannot attest its descendants.
            receipt['cancellation_workload_cleanup_verified'] = False
        receipt['capture_complete'] &= not receipt.get('observer_error', False) and cancelled is None
        encoded = (json.dumps(receipt, separators=(',', ':')) + '\n').encode()
        if len(encoded) > 65536:
            receipt['capture_complete'] = False
            encoded = (json.dumps({
                'diagnostic_only': True, 'suite_exit': receipt['suite_exit'],
                'capture_complete': False, 'receipt_overflow': True,
            }) + '\n').encode()
        try:
            with private_file(directory / 'receipt.json') as output:
                output.write(encoded)
        except OSError:
            pass
        try:
            sys.stdout.buffer.write(encoded)
            if receipt['capture_complete'] and timeline.exists():
                with timeline.open('rb') as source:
                    while chunk := source.read(65536):
                        sys.stdout.buffer.write(chunk)
            sys.stdout.buffer.flush()
        except OSError:
            pass
    code = receipt['suite_exit']
    if code is not None:
        return code if code >= 0 else 128 - code
    return 128 + cancelled if cancelled else 1


if __name__ == '__main__':
    try:
        if len(sys.argv) == 4 and sys.argv[1] == '--control':
            control_cancelled = False

            def interrupted(_signum, _frame):
                global control_cancelled
                control_cancelled = True

            signal.signal(signal.SIGINT, interrupted)
            signal.signal(signal.SIGTERM, interrupted)
            result = control(sys.argv[2], sys.argv[3])
            if control_cancelled:
                if result is None:
                    raise SystemExit(1)
                result['ok'] = False
                if sys.argv[2] == 'create' and result.get('created') and not result.get(
                    'cleanup', {}
                ).get('removed', False):
                    result['cleanup'] = remove(TRACEFS / 'instances' / sys.argv[3])
            if result is not None:
                print(json.dumps(result, separators=(',', ':')), flush=True)
        else:
            raise SystemExit(main())
    except Exception as error:
        print(json.dumps({'observer_control_failed': True, **refusal(error, 'observer-setup')}), flush=True)
        raise SystemExit(1)
