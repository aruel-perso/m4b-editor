"""Author the full-UI installer dialogs that wixl does not compile."""
from pathlib import Path
import shutil
import subprocess


# IDT headers use the Windows Installer schema, including nullable column types.
SCHEMAS = {
    'Dialog': (
        'Dialog HCentering VCentering Width Height Attributes Title Control_First Control_Default Control_Cancel',
        's72 i2 i2 i2 i2 I4 L128 s50 S50 S50', 'Dialog'),
    'Control': (
        'Dialog_ Control Type X Y Width Height Attributes Property Text Control_Next Help',
        's72 s50 s20 i2 i2 i2 i2 I4 S50 L0 S50 L50', 'Dialog_ Control'),
    'ControlEvent': (
        'Dialog_ Control_ Event Argument Condition Ordering',
        's72 s50 s50 l255 S255 I2', 'Dialog_ Control_ Event Argument Condition'),
    'CheckBox': ('Property Value', 's72 L64', 'Property'),
    'EventMapping': (
        'Dialog_ Control_ Event Attribute', 's72 s50 s50 s50', 'Dialog_ Control_ Event'),
    'TextStyle': (
        'TextStyle FaceName Size Color StyleBits', 's72 l32 i2 I4 I2', 'TextStyle'),
    'Binary': ('Name Data', 's72 v0', 'Name'),
}

# These dialogs are for interactive installation of a new ProductCode,
# including major upgrades. Maintenance keeps the existing MSI behavior.
FULL_INSTALL = 'UILevel = 5 AND NOT Installed AND NOT REMOVE AND NOT UPGRADINGPRODUCTCODE AND NOT RESUME'


# MSI style bits shared by the controls in this wizard.
VISIBLE = 0x1
ENABLED = 0x2
NO_PREFIX = 0x20000
LAUNCH_PROPERTY = 'WizardLaunchEditor'
LAUNCH_ACTION = 'WizardStartEditor'
FIXED_ICON_32 = 0x500000


def write_tables(directory, icon):
    rows = {name: [] for name in SCHEMAS}
    rows['TextStyle'] = [
        ('WizardBody', 'Segoe UI', 9, '', ''),
        ('WizardHeading', 'Segoe UI', 14, '', 1),
    ]
    rows['CheckBox'] = [(LAUNCH_PROPERTY, '1')]
    rows['Binary'] = [('WizardIcon', 'app.ico')]
    (directory / 'Binary').mkdir()
    shutil.copyfile(icon, directory / 'Binary/app.ico')

    def dialog(name, heading, message, buttons, *, modal=True, default=None, escape=None, extra=(), welcome_icon=False, first_control=None, last_control_next=None):
        first = buttons[0][0]
        rows['Dialog'].append((name, 50, 50, 370, 195, 3 if modal else 1,
                               'Installation de [ProductName]', first_control or first, default or first,
                               first if escape is None else escape))
        body_x = 76 if welcome_icon else 20
        text_style = VISIBLE | ENABLED | NO_PREFIX
        # Text uses NoPrefix so ampersands in resolved paths are displayed literally.
        rows['Control'].extend([
            (name, 'Heading', 'Text', 20, 17, 330, 24, text_style, '',
             r'{\WizardHeading}' + heading, '', ''),
            (name, 'Message', 'Text', body_x, 51, 350 - body_x, 40, text_style, '', message, '', ''),
            (name, 'Divider', 'Line', 0, 157, 370, 0, 1, '', '', '', ''),
        ])
        if welcome_icon:
            rows['Control'].append((name, 'AppIcon', 'Icon', 20, 48, 45, 45,
                                    VISIBLE | FIXED_ICON_32, '', 'WizardIcon', '', ''))
        rows['Control'].extend((name, *control) for control in extra)
        for index, (control, label, event, argument) in enumerate(buttons):
            following = (last_control_next if index == len(buttons) - 1 and last_control_next
                         else buttons[(index + 1) % len(buttons)][0])
            x = 356 - 78 * (len(buttons) - index)
            rows['Control'].append((name, control, 'PushButton', x, 167, 72, 18,
                                    3, '', label, following, ''))
            rows['ControlEvent'].append((name, control, event, argument, '1', 1))

    cancel = ('Cancel', 'Annuler', 'SpawnDialog', 'WizardConfirmCancel')
    finish = ('Finish', 'Terminer', 'EndDialog', 'Return')
    dialog('WizardWelcome', 'Bienvenue dans M4B Editor',
           'Créez et modifiez vos livres audio M4B : assemblez vos pistes, '
           'organisez les chapitres et personnalisez les métadonnées et la couverture.',
           [('Next', 'Suivant', 'NewDialog', 'WizardReady'), cancel], escape='Cancel', welcome_icon=True)
    dialog('WizardReady', 'Prêt à installer M4B Editor',
           'M4B Editor sera installé pour votre compte Windows dans le dossier suivant.',
           [('Back', 'Retour', 'NewDialog', 'WizardWelcome'),
            ('Install', 'Installer', 'EndDialog', 'Return'), cancel],
           default='Install', escape='Cancel',
           extra=[('Destination', 'Text', 20, 93, 330, 57, VISIBLE | ENABLED | NO_PREFIX, '', '[INSTALLDIR]', '', '')])
    dialog('WizardProgress', 'Installation en cours', 'Veuillez patienter pendant l’installation.',
           [cancel], modal=False, extra=[
               ('Progress', 'ProgressBar', 20, 100, 330, 12, 3, '', '', '', ''),
           ])
    rows['EventMapping'].append(('WizardProgress', 'Progress', 'SetProgress', 'Progress'))
    dialog('WizardConfirmCancel', 'Annuler l’installation',
           'Voulez-vous vraiment annuler l’installation de M4B Editor ?',
           [('Continue', 'Continuer', 'EndDialog', 'Return'),
            ('Abort', 'Oui, annuler', 'EndDialog', 'Exit')])
    dialog('WizardComplete', 'Installation terminée',
           'M4B Editor est disponible dans le menu Démarrer.', [finish],
           escape='', first_control='Launch', last_control_next='Launch', extra=[
               ('Launch', 'CheckBox', 20, 107, 260, 18, 3, LAUNCH_PROPERTY,
                'Lancer M4B Editor', 'Finish', ''),
           ])
    rows['ControlEvent'].remove(('WizardComplete', 'Finish', 'EndDialog', 'Return', '1', 1))
    rows['ControlEvent'].extend([
        ('WizardComplete', 'Finish', 'DoAction', LAUNCH_ACTION, f'{FULL_INSTALL} AND {LAUNCH_PROPERTY} = "1"', 1),
        ('WizardComplete', 'Finish', 'EndDialog', 'Return', '1', 2),
    ])
    dialog('WizardCancelled', 'Installation annulée',
           'L’installation de M4B Editor a été annulée.', [finish])
    dialog('WizardError', 'Échec de l’installation',
           'Windows Installer n’a pas pu terminer l’installation de M4B Editor.', [finish])

    # libmsi reads IDTs as UTF-8. Set the persisted string pool code page
    # explicitly so accented labels render across Windows system locales.
    (directory / '_ForceCodepage.idt').write_bytes(b'\r\n1252\t_ForceCodepage\r\n\r\n')
    for name, (columns, types, keys) in SCHEMAS.items():
        lines = [columns.split(), types.split(), [name, *keys.split()], *rows[name]]
        (directory / f'{name}.idt').write_bytes(
            ''.join('\t'.join(map(str, fields)) + '\r\n' for fields in lines).encode('utf-8'))


def add_wizard(msi, msibuild, directory, icon):
    write_tables(directory, icon)
    # Insert into the existing sequence. Importing a sequence IDT would replace
    # wixl's rows, including costing, upgrade checks and ExecuteAction.
    actions = [
        ('WizardWelcome', 1298),
        ('WizardProgress', 1299),
        ('WizardComplete', -1),
        ('WizardCancelled', -2),
        ('WizardError', -3),
    ]
    command = [msibuild, str(msi.resolve()), '-i', '_ForceCodepage.idt']
    for name in SCHEMAS:
        command.extend(['-i', f'{name}.idt'])
    command.extend(['-q', "INSERT INTO `Property` (`Property`, `Value`) "
                           "VALUES ('DefaultUIFont', 'WizardBody')"])
    # Type 18 invokes the installed File key. Async/no-wait keeps the application
    # independent from the completed installer and runs in the interactive UI process.
    command.extend(['-q', "INSERT INTO `CustomAction` (`Action`, `Type`, `Source`, `Target`) "
                           f"VALUES ('{LAUNCH_ACTION}', 210, 'EditorExecutable', '')"])
    for name, sequence in actions:
        command.extend(['-q', "INSERT INTO `InstallUISequence` (`Action`, `Condition`, `Sequence`) "
                               f"VALUES ('{name}', '{FULL_INSTALL}', {sequence})"])
    if '/' in msibuild:
        command[0] = str(Path(msibuild).resolve())
    subprocess.run(command, cwd=directory, check=True)
