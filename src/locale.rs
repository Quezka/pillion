//! English, Italian and Russian, as every app in the suite has. The English text is the key, and a test
//! fails if a string in the code has no translation or its placeholders don't match.

use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    It,
    Ru,
}

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::It => "it",
            Self::Ru => "ru",
        }
    }

    /// Italian and Russian phones default to their language, and anything else to English.
    pub fn from_code(code: &str) -> Option<Self> {
        match code.get(..2)? {
            "en" => Some(Self::En),
            "it" => Some(Self::It),
            "ru" => Some(Self::Ru),
            _ => None,
        }
    }

    /// The form the routing service wants.
    pub fn route_code(self) -> &'static str {
        match self {
            Self::En => "en-US",
            Self::It => "it-IT",
            Self::Ru => "ru-RU",
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

pub fn current() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::It,
        2 => Lang::Ru,
        _ => Lang::En,
    }
}

fn lookup(lang: Lang, message: &'static str) -> &'static str {
    if lang == Lang::En {
        return message;
    }
    MESSAGES.iter().find(|(en, _, _)| *en == message).map_or(message, |(_, it, ru)| match lang {
        Lang::It => *it,
        _ => *ru,
    })
}

/// A string in the current language.
pub fn t(message: &'static str) -> &'static str {
    lookup(current(), message)
}

/// A string in the current language, with each `{}` filled in turn.
pub fn tf(message: &'static str, values: &[&dyn Display]) -> String {
    let mut out = String::new();
    let mut rest = t(message);
    for value in values {
        let Some((before, after)) = rest.split_once("{}") else {
            break;
        };
        out.push_str(before);
        out.push_str(&value.to_string());
        rest = after;
    }
    out.push_str(rest);
    out
}

/// (English, Italian, Russian)
const MESSAGES: &[(&str, &str, &str)] = &[
    // What the voice says.
    ("Moped connection lost.", "Connessione con il motorino persa.", "Связь с мопедом потеряна."),
    ("The moped refused the pairing code.", "Il motorino ha rifiutato il codice di abbinamento.", "Мопед не принял код сопряжения."),
    ("Battery {} percent, about {} kilometres left.", "Batteria al {} percento, restano circa {} chilometri.", "Заряд {} %, осталось примерно {} км."),
    ("Battery {} percent.", "Batteria al {} percento.", "Заряд {} %."),
    ("No battery reading yet.", "Nessuna lettura della batteria per ora.", "Данных о заряде пока нет."),
    ("Moped connected. {}", "Motorino collegato. {}", "Мопед подключён. {}"),
    ("Battery low.", "Batteria scarica.", "Низкий заряд."),
    ("{} kilometres so far, using {} kilometres per kilowatt hour.", "{} chilometri finora, con {} chilometri per kilowattora.", "Пройдено {} км, расход {} км на киловатт-час."),
    ("Warning. The moped has turned on its system warning.", "Attenzione. Il motorino ha acceso la spia di sistema.", "Внимание. На мопеде загорелся системный индикатор."),
    ("Warning. The moped reports fault {}.", "Attenzione. Il motorino segnala il guasto {}.", "Внимание. Мопед сообщает о неисправности {}."),
    ("Moped switched on.", "Motorino acceso.", "Мопед включён."),
    ("Moped switched off.", "Motorino spento.", "Мопед выключен."),
    ("I couldn't get a route.", "Non sono riuscito a trovare un percorso.", "Не удалось построить маршрут."),
    ("Careful. This route has {} kilometres left, and the battery has about {}.", "Attenzione. Al percorso mancano {} chilometri e alla batteria ne restano circa {}.", "Осторожно. До конца маршрута {} км, а заряда хватит примерно на {}."),
    ("Careful. This route has {} kilometres left, and the battery is too low to say if it will make it.", "Attenzione. Al percorso mancano {} chilometri e la batteria è troppo scarica per dire se basterà.", "Осторожно. До конца маршрута {} км, а заряд слишком мал, чтобы сказать, хватит ли его."),
    ("You have arrived.", "Sei arrivato.", "Вы прибыли."),
    ("Off route. Finding a new way.", "Fuori percorso. Cerco un'altra strada.", "Вы сошли с маршрута. Ищу новый путь."),
    ("Call from {}.", "Chiamata da {}.", "Звонит {}."),
    ("Incoming call.", "Chiamata in arrivo.", "Входящий звонок."),
    ("Missed call.", "Chiamata persa.", "Пропущенный звонок."),
    ("Voice mode on. {}", "Voce attiva. {}", "Голос включён. {}"),
    ("The moped is not paired.", "Il motorino non è abbinato.", "Мопед не сопряжён."),
    ("The moped is not connected.", "Il motorino non è collegato.", "Мопед не подключён."),
    ("This ride: {} kilometres at {} kilometres per kilowatt hour.", "Questo viaggio: {} chilometri a {} chilometri per kilowattora.", "Эта поездка: {} км, расход {} км на киловатт-час."),
    ("In {} metres, {}", "Tra {} metri, {}", "Через {} м: {}"),
    ("{} kilometres and {} minutes to go.", "Mancano {} chilometri e {} minuti.", "Осталось {} км и {} мин."),
    ("The moped has a warning on.", "Il motorino ha una spia accesa.", "На мопеде горит предупреждение."),
    ("Ride finished. {} kilometres at {} kilometres per kilowatt hour.", "Viaggio finito. {} chilometri a {} chilometri per kilowattora.", "Поездка окончена. {} км, расход {} км на киловатт-час."),
    ("Route ready, {} kilometres, about {} minutes. {}", "Percorso pronto, {} chilometri, circa {} minuti. {}", "Маршрут готов: {} км, примерно {} мин. {}"),
    ("{} more from {}.", "Altri {} da {}.", "Ещё {} от {}."),
    ("This is how I'll sound on your ride. If you can hear me in your earpiece, you're all set.", "Ecco come mi sentirai in viaggio. Se mi senti nell'auricolare, è tutto a posto.", "Так я буду звучать в поездке. Если вы слышите меня в наушнике, всё готово."),
    ("Bluetooth is off.", "Il Bluetooth è spento.", "Bluetooth выключен."),
    // Navigation status.
    ("Searching...", "Ricerca in corso...", "Поиск..."),
    ("That code didn't work", "Questo codice non va", "Этот код не подошёл"),
    ("It should have 24 characters, or 20 if it's already unscrambled.", "Deve avere 24 caratteri, o 20 se è già in chiaro.", "В нём должно быть 24 символа или 20, если он уже расшифрован."),
    ("Nothing found.", "Nessun risultato.", "Ничего не найдено."),
    ("Search failed. Check the phone's internet connection.", "Ricerca non riuscita. Controlla la connessione del telefono.", "Поиск не удался. Проверьте интернет на телефоне."),
    ("Waiting for GPS...", "In attesa del GPS...", "Ожидание GPS..."),
    ("Finding a route...", "Cerco un percorso...", "Поиск маршрута..."),
    ("A moped can't get right to that spot, so this goes to the nearest road.", "Un motorino non può arrivare proprio lì, quindi si va alla strada più vicina.", "Мопед не проедет к самому месту, поэтому маршрут идёт до ближайшей дороги."),
    ("No route: {}", "Nessun percorso: {}", "Нет маршрута: {}"),
    ("Where to?", "Dove vai?", "Куда едем?"),
    ("Address or place", "Indirizzo o luogo", "Адрес или место"),
    ("Pairing code", "Codice di abbinamento", "Код сопряжения"),
    ("24 characters from the QR sticker", "24 caratteri dall'adesivo QR", "24 символа с QR-наклейки"),
    // The demo ride.
    ("Demo destination", "Destinazione demo", "Демо-пункт назначения"),
    ("Head north", "Vai verso nord", "Двигайтесь на север"),
    ("Turn right", "Gira a destra", "Поверните направо"),
    ("Bear left", "Tieni la sinistra", "Держитесь левее"),
    ("Arrive", "Arrivo", "Прибытие"),
    ("{} onto {}.", "{} in {}.", "{}: {}."),
    ("{}.", "{}.", "{}."),
    ("See you at six!", "Ci vediamo alle sei!", "Увидимся в шесть!"),
    // Units.
    ("{} km", "{} km", "{} км"),
    ("{} m", "{} m", "{} м"),
    ("{} kW", "{} kW", "{} кВт"),
    ("{} kWh", "{} kWh", "{} кВт·ч"),
    ("{} km/kWh", "{} km/kWh", "{} км/кВт·ч"),
    ("{} h", "{} h", "{} ч"),
    ("km/h", "km/h", "км/ч"),
    // Screens.
    ("Ride", "Viaggio", "Поездка"),
    ("Route", "Percorso", "Маршрут"),
    ("Voice", "Voce", "Голос"),
    ("Moped", "Motorino", "Мопед"),
    ("Alerts", "Avvisi", "Оповещения"),
    ("Settings", "Impostazioni", "Настройки"),
    ("Demo ride", "Viaggio demo", "Демо-поездка"),
    ("No data", "Nessun dato", "Нет данных"),
    ("Connected", "Collegato", "Подключён"),
    ("Searching", "Ricerca", "Поиск"),
    ("Connecting", "Collegamento", "Подключение"),
    ("Code refused", "Codice rifiutato", "Код отклонён"),
    ("Not connected", "Non collegato", "Не подключён"),
    ("Not paired", "Non abbinato", "Не сопряжён"),
    ("Bluetooth is off", "Il Bluetooth è spento", "Bluetooth выключен"),
    ("Turn it on so Pillion can reach the moped.", "Accendilo perché Pillion raggiunga il motorino.", "Включите его, чтобы Pillion связался с мопедом."),
    ("Turn on Bluetooth", "Accendi il Bluetooth", "Включить Bluetooth"),
    ("Bluetooth isn't allowed", "Bluetooth non consentito", "Bluetooth запрещён"),
    ("Pillion needs the Nearby devices permission to reach the moped.", "Pillion ha bisogno del permesso Dispositivi nelle vicinanze per raggiungere il motorino.", "Pillion нужно разрешение «Устройства поблизости», чтобы связаться с мопедом."),
    ("Open app settings", "Apri le impostazioni dell'app", "Открыть настройки приложения"),
    ("Location is off", "La posizione è spenta", "Геолокация выключена"),
    ("Directions need it to follow you along the route.", "Serve alle indicazioni per seguirti lungo il percorso.", "Она нужна, чтобы вести вас по маршруту."),
    ("Turn on location", "Accendi la posizione", "Включить геолокацию"),
    ("This phone can't speak yet", "Questo telefono non può ancora parlare", "Этот телефон пока не умеет говорить"),
    ("It has no working speech engine. Install Speech Services by Google from the Play Store, then reopen Pillion.", "Non ha un motore vocale funzionante. Installa Servizi di riconoscimento vocale di Google dal Play Store, poi riapri Pillion.", "На нём нет рабочего синтезатора речи. Установите «Распознавание и синтез речи от Google» из Play Маркета и снова откройте Pillion."),
    ("Get ready to ride", "Preparati a partire", "Готовимся к поездке"),
    ("Pillion rides along and tells you what's happening, so your eyes stay on the road.", "Pillion viaggia con te e ti dice cosa succede, così gli occhi restano sulla strada.", "Pillion едет с вами и рассказывает, что происходит, а ваши глаза остаются на дороге."),
    ("Pair your moped", "Abbina il motorino", "Сопрягите мопед"),
    ("The moped refused this code", "Il motorino ha rifiutato questo codice", "Мопед не принял этот код"),
    ("Type the 24 characters from its QR sticker. A QR reader app shows them.", "Scrivi i 24 caratteri dell'adesivo QR. Li mostra un'app per leggere i QR.", "Введите 24 символа с QR-наклейки. Их покажет приложение для чтения QR-кодов."),
    ("Allow notifications", "Consenti le notifiche", "Разрешите уведомления"),
    ("So messages can be read to you and light up the dash.", "Così i messaggi ti vengono letti e accendono il cruscotto.", "Чтобы сообщения зачитывались вслух и зажигали индикаторы на приборной панели."),
    ("Test your earpiece", "Prova l'auricolare", "Проверьте наушник"),
    ("Pillion says a sentence so you know you'll hear it.", "Pillion dice una frase, così sai che la sentirai.", "Pillion произнесёт фразу, чтобы вы знали, что слышите его."),
    ("Try a demo ride", "Prova un viaggio demo", "Попробуйте демо-поездку"),
    ("See and hear everything first, with invented figures.", "Guarda e ascolta tutto prima, con dati inventati.", "Сначала посмотрите и послушайте всё на вымышленных данных."),
    ("Plan a route", "Pianifica un percorso", "Проложить маршрут"),
    ("Spoken directions, on roads a moped can use.", "Indicazioni a voce, su strade adatte a un motorino.", "Голосовые подсказки по дорогам, доступным для мопеда."),
    ("this ride", "questo viaggio", "эта поездка"),
    ("energy", "energia", "энергия"),
    ("power", "potenza", "мощность"),
    ("battery", "batteria", "заряд"),
    ("too low to estimate", "troppo poca per stimarla", "слишком мало для оценки"),
    ("range left", "autonomia residua", "запас хода"),
    ("this ride's consumption", "consumo di questo viaggio", "расход в этой поездке"),
    ("consumption: measuring", "consumo: in misura", "расход: измеряется"),
    ("Off route, finding a new way", "Fuori percorso, cerco un'altra strada", "Вне маршрута, ищу новый путь"),
    ("{} to go, {} min", "mancano {}, {} min", "осталось {}, {} мин"),
    ("Lights on the moped's dash", "Spie sul cruscotto del motorino", "Индикаторы на панели мопеда"),
    ("Coming up", "Più avanti", "Далее"),
    ("Stop navigation", "Ferma la navigazione", "Остановить навигацию"),
    ("Nowhere yet", "Ancora nessun posto", "Пока никуда"),
    ("Search for an address or a place. Places you've been will wait here for a quick start.", "Cerca un indirizzo o un luogo. I posti già visti resteranno qui per ripartire al volo.", "Найдите адрес или место. Посещённые места останутся здесь для быстрого старта."),
    ("Recent places", "Luoghi recenti", "Недавние места"),
    ("Planning a route sends your position and destination to free OpenStreetMap services. Routes and places: © OpenStreetMap contributors", "Pianificare un percorso invia posizione e destinazione ai servizi gratuiti di OpenStreetMap. Percorsi e luoghi: © OpenStreetMap contributors", "При построении маршрута ваше положение и пункт назначения отправляются в бесплатные сервисы OpenStreetMap. Маршруты и места: © OpenStreetMap contributors"),
    ("Tell me how things are", "Dimmi come va", "Расскажи, как дела"),
    ("Tap here, or press your earpiece's button", "Tocca qui o premi il tasto dell'auricolare", "Нажмите здесь или кнопку на наушнике"),
    ("Repeat", "Ripeti", "Повторить"),
    ("Slower", "Più lenta", "Медленнее"),
    ("Faster", "Più veloce", "Быстрее"),
    ("Speak to me", "Parlami", "Говорить со мной"),
    ("What to tell me about", "Di cosa parlarmi", "О чём рассказывать"),
    ("Battery and range", "Batteria e autonomia", "Заряд и запас хода"),
    ("Consumption", "Consumi", "Расход"),
    ("Directions", "Indicazioni", "Маршрут"),
    ("Calls", "Chiamate", "Звонки"),
    ("Warnings are always spoken. During a call, only directions and warnings are.", "Gli avvisi si sentono sempre. Durante una chiamata, solo indicazioni e avvisi.", "Предупреждения звучат всегда. Во время звонка только подсказки маршрута и предупреждения."),
    ("Last said", "Ultime frasi", "Последние фразы"),
    ("Nothing yet.", "Ancora niente.", "Пока ничего."),
    ("Battery {}: {}%", "Batteria {}: {}%", "Батарея {}: {}%"),
    ("voltage", "tensione", "напряжение"),
    ("current", "corrente", "ток"),
    ("temperature", "temperatura", "температура"),
    ("stored", "carica", "запас"),
    ("health", "salute", "состояние"),
    ("to charge", "alla ricarica", "до зарядки"),
    ("odometer", "contachilometri", "пробег"),
    ("12 V battery", "batteria 12 V", "батарея 12 В"),
    ("motor", "motore", "мотор"),
    ("About {} km on a full charge", "Circa {} km con una carica completa", "Примерно {} км на полном заряде"),
    ("Model number {}", "Numero modello {}", "Номер модели {}"),
    ("Warning on", "Spia accesa", "Горит предупреждение"),
    ("Warning on: {}", "Spia accesa: {}", "Горит предупреждение: {}"),
    ("No warnings", "Nessuna spia", "Предупреждений нет"),
    ("End the demo ride", "Termina il viaggio demo", "Завершить демо-поездку"),
    ("This is a demo ride", "Questo è un viaggio demo", "Это демо-поездка"),
    ("These figures are invented to show the app. They aren't your moped's.", "Questi dati sono inventati per mostrare l'app. Non sono del tuo motorino.", "Эти данные вымышлены для показа приложения. Это не данные вашего мопеда."),
    ("Enter pairing code", "Inserisci il codice", "Ввести код"),
    ("Moped paired", "Motorino abbinato", "Мопед сопряжён"),
    ("Connect", "Collega", "Подключить"),
    ("Tap again to forget this moped", "Tocca ancora per dimenticarlo", "Нажмите ещё раз, чтобы забыть"),
    ("Forget this moped", "Dimentica questo motorino", "Забыть этот мопед"),
    ("Looking for the moped. Turn it on and stay close.", "Cerco il motorino. Accendilo e resta vicino.", "Ищу мопед. Включите его и будьте рядом."),
    ("Connecting to the moped...", "Mi collego al motorino...", "Подключаюсь к мопеду..."),
    ("Signing in to the moped...", "Accesso al motorino...", "Вход в мопед..."),
    ("Use {}", "Usa {}", "Использовать {}"),
    ("Cancel", "Annulla", "Отмена"),
    ("Connected to the moped", "Collegato al motorino", "Подключён к мопеду"),
    ("Disconnect", "Scollega", "Отключить"),
    ("Let Pillion see notifications", "Fai vedere le notifiche a Pillion", "Разрешите Pillion видеть уведомления"),
    ("Android has to allow this. Message text is only read aloud on your phone and is never sent anywhere. If Android calls it a restricted setting, open Pillion's App info, tap the three dots, then Allow restricted settings.", "Android deve consentirlo. Il testo dei messaggi viene solo letto sul telefono e non va da nessuna parte. Se Android parla di impostazione con restrizioni, apri Informazioni app di Pillion, tocca i tre puntini, poi Consenti impostazioni con restrizioni.", "Это должен разрешить Android. Текст сообщений только зачитывается на телефоне и никуда не отправляется. Если Android называет это ограниченной настройкой, откройте сведения о приложении Pillion, нажмите три точки и выберите «Разрешить ограниченные настройки»."),
    ("Open the Android setting", "Apri l'impostazione di Android", "Открыть настройку Android"),
    ("Read notifications aloud", "Leggi le notifiche ad alta voce", "Читать уведомления вслух"),
    ("Off", "No", "Выкл."),
    ("Who", "Chi", "Кто"),
    ("Full text", "Tutto", "Весь текст"),
    ("Several messages in a row from one app are told as a single count.", "Più messaggi di fila dalla stessa app diventano un solo conteggio.", "Несколько сообщений подряд от одного приложения озвучиваются одним числом."),
    ("Show calls on the dash", "Mostra le chiamate sul cruscotto", "Показывать звонки на панели"),
    ("Apps", "App", "Приложения"),
    ("The dash can only light up, it can't show message text. Each app below gets a light, up to six.", "Il cruscotto può solo accendere spie, non mostrare il testo. Ogni app qui sotto ha una spia, fino a sei.", "Панель умеет только зажигать индикаторы, текст она не показывает. Каждому приложению ниже достаётся индикатор, всего до шести."),
    ("Apps appear here after their first notification.", "Le app compaiono qui dopo la prima notifica.", "Приложения появятся здесь после первого уведомления."),
    ("no light", "nessuna spia", "без индикатора"),
    ("light {}", "spia {}", "индикатор {}"),
    ("Recent", "Recenti", "Недавние"),
    ("Appearance", "Aspetto", "Оформление"),
    ("System", "Sistema", "Система"),
    ("Light", "Chiaro", "Светлое"),
    ("Dark", "Scuro", "Тёмное"),
    ("High contrast", "Contrasto elevato", "Высокая контрастность"),
    ("Language", "Lingua", "Язык"),
    ("Spoken directions follow this too.", "Anche le indicazioni a voce la seguono.", "Голосовые подсказки тоже следуют этой настройке."),
    ("Version {}", "Versione {}", "Версия {}"),
    ("The seat behind the rider. A companion for your electric moped, part of the Quezka suite.", "Il sellino del passeggero. Un compagno per il tuo motorino elettrico, parte della suite Quezka.", "Сиденье позади водителя. Спутник для вашего электромопеда из набора приложений Quezka."),
    ("Manrope font (SIL Open Font License), Material Symbols (Apache 2.0), routes and places © OpenStreetMap contributors.", "Font Manrope (SIL Open Font License), Material Symbols (Apache 2.0), percorsi e luoghi © OpenStreetMap contributors.", "Шрифт Manrope (SIL Open Font License), Material Symbols (Apache 2.0), маршруты и места © OpenStreetMap contributors."),
    ("OK", "OK", "ОК"),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `t("...")` and `tf("...")` in a source file.
    fn strings_in(source: &str) -> Vec<String> {
        let mut found = Vec::new();
        for opener in ["t(\"", "tf(\""] {
            let mut rest = source;
            while let Some(at) = rest.find(opener) {
                let before = rest[..at].chars().next_back();
                rest = &rest[at + opener.len()..];
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let end = rest.find('"').expect("a string ends");
                found.push(rest[..end].to_string());
            }
        }
        found
    }

    #[test]
    fn every_string_in_the_code_is_translated() {
        let sources = [
            include_str!("ui.rs"),
            include_str!("engine.rs"),
            include_str!("nav.rs"),
            include_str!("demo.rs"),
        ];
        let mut count = 0;
        for source in sources {
            for found in strings_in(source) {
                assert!(MESSAGES.iter().any(|(en, _, _)| *en == found), "no translation for: {found}");
                count += 1;
            }
        }
        assert!(count > 100, "only saw {count} strings");
    }

    #[test]
    fn placeholders_match_and_nothing_is_left_empty() {
        for (en, it, ru) in MESSAGES {
            let holes = en.matches("{}").count();
            assert_eq!(it.matches("{}").count(), holes, "Italian: {en}");
            assert_eq!(ru.matches("{}").count(), holes, "Russian: {en}");
            assert!(!it.is_empty() && !ru.is_empty(), "{en}");
        }
    }

    #[test]
    fn no_message_is_listed_twice() {
        for (i, (en, _, _)) in MESSAGES.iter().enumerate() {
            assert!(!MESSAGES[i + 1..].iter().any(|(other, _, _)| other == en), "twice: {en}");
        }
    }

    #[test]
    fn languages_are_looked_up_and_fill_in_order() {
        assert_eq!(lookup(Lang::It, "Ride"), "Viaggio");
        assert_eq!(lookup(Lang::Ru, "Ride"), "Поездка");
        assert_eq!(lookup(Lang::En, "Ride"), "Ride");
        assert_eq!(tf("Battery {}: {}%", &[&1, &64]), "Battery 1: 64%");
        assert_eq!(Lang::from_code("it-IT"), Some(Lang::It));
        assert_eq!(Lang::from_code("de"), None);
        assert_eq!(Lang::from_code(""), None);
    }
}
