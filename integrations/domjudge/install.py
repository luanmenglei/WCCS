"""Install the version-pinned WCCS controller and clock, with source backup."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
from contest_timing import patched


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--domserver', required=True, type=Path)
    parser.add_argument('--backup-directory', required=True, type=Path)
    parser.add_argument('--source-webapp', required=True, type=Path)
    arguments = parser.parse_args()
    if os.geteuid() != 0:
        raise SystemExit('Run as root on the designated DOMjudge server.')
    os.umask(0o077)
    root = arguments.domserver.resolve(strict=True)
    source = Path(__file__).resolve().parent
    static = (root / 'etc/domserver-static.php').read_text()
    if "define('DOMJUDGE_VERSION', '9.0.2')" not in static:
        raise SystemExit('This extension requires the fixed DOMjudge 9.0.2 release.')
    template = root / 'webapp/templates/partials/menu_countdown.html.twig'
    original = template.read_bytes()
    contest = root / 'webapp/src/Entity/Contest.php'
    original_contest = contest.read_bytes()
    patched_contest = patched(original_contest)
    baseline = '71c901f8048dcd6e3e2d910b09e2aff07c9ba188c630f3629e9e47f8ca957600'
    if hashlib.sha256(original).hexdigest() != baseline:
        raise SystemExit('Countdown template differs from the verified baseline. Preserve and review the existing changes.')
    subprocess.run(['php', '-l', str(source / 'WccsControlController.php')], check=True)
    backup = arguments.backup_directory.resolve() / datetime.now(timezone.utc).strftime('wccs-control-%Y%m%dT%H%M%SZ')
    backup.mkdir(parents=True, mode=0o700)
    shutil.copy2(template, backup / 'menu_countdown.html.twig')
    shutil.copy2(contest, backup / 'Contest.php')
    autoload = root / 'webapp/vendor/composer'
    shutil.copytree(autoload, backup / 'composer-autoload')
    shutil.copy2(root / 'webapp/vendor/autoload.php', backup / 'autoload.php')
    composer_files = [root / 'webapp/composer.json', root / 'webapp/composer.lock']
    if any(path.exists() for path in composer_files):
        raise SystemExit('Installed Composer metadata already exists. Review before updating.')
    targets = {
        source / 'WccsControlController.php': root / 'webapp/src/Controller/API/WccsControlController.php',
        source / 'wccs-clock.js': root / 'webapp/public/js/wccs-clock.js',
    }
    if any(target.exists() for target in targets.values()):
        raise SystemExit('Extension target already exists. Review the installed version before updating.')
    # The two extension tables are additive and are retained after rollback for audit.
    subprocess.run(['mariadb', 'domjudge'], input=(source / 'schema.sql').read_bytes(), check=True)
    appended = b'''\n{% if contest is not null %}
<script>window.wccsControlUrl = {{ path('v4_wccs_control_status', {'contestId': contest.externalid})|json_encode|raw }};</script>
<script src="{{ asset('js/wccs-clock.js') }}"></script>
{% endif %}
'''
    try:
        for item, target in targets.items():
            shutil.copy2(item, target)
            target.chmod(0o644)
        template.write_bytes(original + appended)
        contest.write_bytes(patched_contest)
        subprocess.run(['php', '-l', str(contest)], check=True)
        for path in composer_files:
            shutil.copy2(arguments.source_webapp / path.name, path)
        subprocess.run(['composer', 'dump-autoload', '--classmap-authoritative', '--no-dev', '--no-scripts', '--no-plugins'],
                       cwd=root / 'webapp', env=os.environ | {'COMPOSER_ALLOW_SUPERUSER': '1'}, check=True)
        for path in autoload.rglob('*'):
            path.chmod(0o755 if path.is_dir() else 0o644)
        (root / 'webapp/vendor/autoload.php').chmod(0o644)
        subprocess.run(['runuser', '-u', 'www-data', '--', str(root / 'bin/dj_console'), 'cache:clear', '--env=prod'], check=True)
        subprocess.run(['runuser', '-u', 'www-data', '--', str(root / 'bin/dj_console'), 'debug:router', 'v4_wccs_control_status', '--env=prod'], check=True)
        subprocess.run(['systemctl', 'reload', 'php8.3-fpm'], check=True)
    except Exception:
        template.write_bytes(original)
        contest.write_bytes(original_contest)
        for target in targets.values():
            target.unlink(missing_ok=True)
        shutil.copytree(backup / 'composer-autoload', autoload, dirs_exist_ok=True)
        shutil.copy2(backup / 'autoload.php', root / 'webapp/vendor/autoload.php')
        subprocess.run(['runuser', '-u', 'www-data', '--', str(root / 'bin/dj_console'), 'cache:clear', '--env=prod'], check=False)
        subprocess.run(['systemctl', 'reload', 'php8.3-fpm'], check=False)
        raise
    finally:
        for path in composer_files:
            path.unlink(missing_ok=True)
    report = {'version': 1, 'domjudge': '9.0.2', 'backup': str(backup),
              'files': {str(target): hashlib.sha256(target.read_bytes()).hexdigest() for target in targets.values()}}
    (backup / 'installation.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report))


if __name__ == '__main__':
    main()
