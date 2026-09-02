//! Compact UPSC taxonomy identifiers and the shared label registry.
//!
//! Question JSON stores numeric enum IDs. Human-readable labels and descriptions
//! live once in `static/upsc/taxonomy.json` and are resolved during import.

use std::collections::HashSet;
use std::sync::OnceLock;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const MAX_SUBTAGS: usize = 4;
pub const TAXONOMY_VERSION: u16 = 4;
const LABELS_JSON: &str = include_str!("../../static/upsc/taxonomy.json");

macro_rules! numeric_enum {
    ($name:ident { $($variant:ident = $value:literal,)+ }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(u16)]
        pub enum $name {
            $($variant = $value,)+
        }

        impl $name {
            pub const fn id(self) -> u16 { self as u16 }
        }

        impl TryFrom<u16> for $name {
            type Error = String;

            fn try_from(value: u16) -> Result<Self, Self::Error> {
                match value {
                    $($value => Ok(Self::$variant),)+
                    _ => Err(format!("Unknown {} ID: {value}", stringify!($name))),
                }
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_u16(self.id())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = u16::deserialize(deserializer)?;
                Self::try_from(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

numeric_enum!(MainTag {
    Sports = 0,
    Awards = 1,
    Cinema = 2,
    ScientificHeritage = 3,
    AncientIndia = 4,
    MedievalIndia = 5,
    ModernIndia = 6,
    Nationalism = 7,
    FreedomStruggle = 8,
    PostIndependenceIndia = 9,
    PhysicalGeography = 10,
    HumanGeography = 11,
    EconomicGeography = 12,
    IndianGeography = 13,
    WorldGeography = 14,
    Biogeography = 15,
    NaturalResources = 16,
    CriticalMinerals = 17,
    Constitution = 18,
    Parliament = 19,
    Executive = 20,
    Judiciary = 21,
    Federalism = 22,
    Elections = 23,
    ConstitutionalBodies = 24,
    StatutoryBodies = 25,
    RegulatoryBodies = 26,
    LocalSelfGovernment = 27,
    PartySystem = 28,
    PublicPolicy = 29,
    Rights = 30,
    CivilServices = 31,
    Accountability = 32,
    EconomicDevelopment = 33,
    EconomicGrowth = 34,
    EconomicPlanning = 35,
    Microeconomics = 36,
    BlackMoney = 37,
    FiscalPolicy = 38,
    Taxation = 39,
    MonetaryPolicy = 40,
    Banking = 41,
    FinancialMarkets = 42,
    Currency = 43,
    Insurance = 44,
    ForeignInvestment = 45,
    BalanceOfPayments = 46,
    Inflation = 47,
    InternationalTrade = 48,
    Industry = 49,
    CorporateGovernance = 50,
    Infrastructure = 51,
    Labour = 52,
    Employment = 53,
    FinancialManagement = 54,
    DigitalEconomy = 55,
    SustainableDevelopment = 56,
    Poverty = 57,
    SocialInclusion = 58,
    Gender = 59,
    TribalCommunities = 60,
    Demography = 61,
    Education = 62,
    Health = 63,
    SocialSecurity = 64,
    WelfareSchemes = 65,
    EnvironmentalEcology = 66,
    Biodiversity = 67,
    ClimateChange = 68,
    Pollution = 69,
    Conservation = 70,
    EnvironmentalLaw = 71,
    Forests = 72,
    Physics = 73,
    Chemistry = 74,
    Biology = 75,
    HumanEvolution = 76,
    Biotechnology = 77,
    InformationTechnology = 78,
    ArtificialIntelligence = 79,
    SpaceTechnology = 80,
    NuclearTechnology = 81,
    DefenceTechnology = 82,
    TechnologyApplications = 83,
    ScientificResearch = 84,
    Energy = 85,
    Agriculture = 86,
    CroppingPatterns = 87,
    Irrigation = 88,
    FarmInputs = 89,
    AgriculturalMarkets = 90,
    FoodSecurity = 91,
    FoodProcessing = 92,
    Fisheries = 93,
    LandReforms = 94,
    WaterResources = 95,
    InternationalInstitutions = 96,
    BilateralRelations = 97,
    RegionalGroupings = 98,
    ForeignPolicy = 99,
    Geopolitics = 100,
    InternalSecurity = 101,
    CyberSecurity = 102,
    BorderSecurity = 103,
    Terrorism = 104,
    OrganisedCrime = 105,
    CriminalJustice = 106,
    EthicalDilemmas = 107,
    DisasterManagement = 108,
    ArtForms = 109,
    Music = 110,
    Dance = 111,
    Painting = 112,
    Crafts = 113,
    Sculpture = 114,
    Archaeology = 115,
    ReligiousTraditions = 116,
    PoliticalIdeologies = 117,
    Literature = 118,
    Architecture = 119,
    ColonialPolicy = 120,
    SocialReformMovements = 121,
    ReadingComprehension = 122,
    InterpersonalSkills = 123,
    CommunicationSkills = 124,
    LogicalReasoning = 125,
    AnalyticalAbility = 126,
    DecisionMaking = 127,
    ProblemSolving = 128,
    GeneralMentalAbility = 129,
    BasicNumeracy = 130,
    DataInterpretation = 131,
    PostIndependenceConsolidation = 132,
    DemocraticRevolutions = 133,
    Enlightenment = 134,
    IndustrialRevolution = 135,
    WorldWars = 136,
    Decolonisation = 137,
    SocialStructure = 138,
    Caste = 139,
    Family = 140,
    Diversity = 141,
    Women = 142,
    Population = 143,
    DevelopmentalIssues = 144,
    Urbanisation = 145,
    Globalisation = 146,
    Communalism = 147,
    Regionalism = 148,
    Secularism = 149,
    Geomorphology = 150,
    Climatology = 151,
    Oceanography = 152,
    IndustrialLocation = 153,
    GeophysicalPhenomena = 154,
    EnvironmentalChange = 155,
    ConstitutionalAmendment = 156,
    SeparationOfPowers = 157,
    DisputeResolution = 158,
    ComparativeConstitutionalism = 159,
    StateLegislature = 160,
    JudicialReview = 161,
    JudicialAppointments = 162,
    AntiDefection = 163,
    Representation = 164,
    PressureGroups = 165,
    RepresentationOfThePeopleAct = 166,
    QuasiJudicialBodies = 167,
    GovernmentPolicy = 168,
    DevelopmentProcess = 169,
    NonGovernmentalOrganisations = 170,
    SelfHelpGroups = 171,
    VulnerableSections = 172,
    HealthServices = 173,
    EducationServices = 174,
    HumanResources = 175,
    Hunger = 176,
    Transparency = 177,
    EGovernance = 178,
    CitizenCharter = 179,
    NeighbourhoodRelations = 180,
    GlobalGroupings = 181,
    InternationalAgreements = 182,
    IndianDiaspora = 183,
    InclusiveGrowth = 184,
    ExternalSector = 185,
    GovernmentBudgeting = 186,
    AgriculturalSupplyChains = 187,
    AgriculturalTechnology = 188,
    FarmSubsidies = 189,
    MinimumSupportPrice = 190,
    PublicDistributionSystem = 191,
    BufferStocks = 192,
    OrganicFarming = 193,
    WaterManagement = 194,
    Liberalisation = 195,
    IndustrialPolicy = 196,
    InvestmentModels = 197,
    HealthTechnology = 198,
    Nanotechnology = 199,
    IntellectualPropertyRights = 200,
    EnvironmentalDegradation = 201,
    EnvironmentalImpactAssessment = 202,
    GeophysicalHazards = 203,
    Extremism = 204,
    ExternalSecurityThreats = 205,
    MediaSecurity = 206,
    MoneyLaundering = 207,
    SecurityForces = 208,
    Ethics = 209,
    HumanValues = 210,
    Attitude = 211,
    Persuasion = 212,
    Aptitude = 213,
    Integrity = 214,
    Impartiality = 215,
    Objectivity = 216,
    DedicationToPublicService = 217,
    Empathy = 218,
    Tolerance = 219,
    Compassion = 220,
    EmotionalIntelligence = 221,
    MoralThinkers = 222,
    PublicServiceValues = 223,
    ConflictOfInterest = 224,
    EthicalGovernance = 225,
    Probity = 226,
    RightToInformation = 227,
    CodeOfEthics = 228,
    CodeOfConduct = 229,
    WorkCulture = 230,
    ServiceDelivery = 231,
    PublicFunds = 232,
    Corruption = 233,
    AnthropologicalScope = 234,
    SocioculturalAnthropology = 235,
    BiologicalAnthropology = 236,
    ArchaeologicalAnthropology = 237,
    LinguisticAnthropology = 238,
    Primatology = 239,
    Palaeoanthropology = 240,
    MolecularAnthropology = 241,
    PrehistoricArchaeology = 242,
    Culture = 243,
    Society = 244,
    Marriage = 245,
    Kinship = 246,
    EconomicAnthropology = 247,
    PoliticalAnthropology = 248,
    AnthropologyOfReligion = 249,
    AnthropologicalTheory = 250,
    Language = 251,
    AnthropologicalResearchMethods = 252,
    HumanGenetics = 253,
    MendelianGenetics = 254,
    PopulationGenetics = 255,
    ChromosomalDisorders = 256,
    Race = 257,
    EcologicalAnthropology = 258,
    EpidemiologicalAnthropology = 259,
    HumanGrowth = 260,
    Fertility = 261,
    DemographicTheory = 262,
    AppliedAnthropology = 263,
    IndianPrehistory = 264,
    IndianPalaeoanthropology = 265,
    Ethnoarchaeology = 266,
    IndianDemography = 267,
    IndianSocialSystem = 268,
    SacredComplex = 269,
    ReligiousChange = 270,
    IndianAnthropology = 271,
    IndianVillage = 272,
    Minorities = 273,
    SocioculturalChange = 274,
    TribalDemography = 275,
    TribalProblems = 276,
    TribalDisplacement = 277,
    ConstitutionalSafeguards = 278,
    TribalChange = 279,
    Ethnicity = 280,
    TribalReligion = 281,
    TribeStateRelations = 282,
    TribalAdministration = 283,
    TribalDevelopment = 284,
    RuralDevelopment = 285,
    FinancialAccounting = 286,
    CostAccounting = 287,
    Auditing = 288,
    FinancialInstitutions = 289,
    OrganisationTheory = 290,
    OrganisationalBehaviour = 291,
    HumanResourceManagement = 292,
    IndustrialRelations = 293,
    WelfareEconomics = 294,
    Macroeconomics = 295,
    MonetaryEconomics = 296,
    PublicFinance = 297,
    HumanDevelopment = 298,
    EnvironmentalEconomics = 299,
    ColonialEconomy = 300,
    AgriculturalDevelopment = 301,
    IndustrialDevelopment = 302,
    NationalIncome = 303,
    AgriculturalPolicy = 304,
    TradePolicy = 305,
    ExchangeRatePolicy = 306,
    EnvironmentalGeography = 307,
    PopulationGeography = 308,
    SettlementGeography = 309,
    RegionalPlanning = 310,
    GeographicalModels = 311,
    PhysicalSettingOfIndia = 312,
    AgriculturalGeography = 313,
    IndustrialGeography = 314,
    TransportGeography = 315,
    CommunicationGeography = 316,
    TradeGeography = 317,
    CulturalGeographyOfIndia = 318,
    SettlementGeographyOfIndia = 319,
    RegionalDevelopment = 320,
    PoliticalGeography = 321,
    ContemporaryGeographicalIssues = 322,
    MapInterpretation = 323,
    HistoricalMapWork = 324,
    HistoricalSources = 325,
    Prehistory = 326,
    IndusValleyCivilisation = 327,
    MegalithicCultures = 328,
    VedicPeriod = 329,
    Mahajanapadas = 330,
    MauryanEmpire = 331,
    PostMauryanIndia = 332,
    EarlyPeninsularIndia = 333,
    GuptaAge = 334,
    EarlyIndianCulture = 335,
    EarlyMedievalIndia = 336,
    MedievalCulturalTraditions = 337,
    DelhiSultanate = 338,
    KhaljiDynasty = 339,
    TughlaqDynasty = 340,
    SultanateSociety = 341,
    VijayanagaraEmpire = 342,
    MughalFoundation = 343,
    MughalConsolidation = 344,
    SeventeenthCenturyMughalEmpire = 345,
    MughalEconomy = 346,
    MughalCulture = 347,
    EighteenthCenturyIndia = 348,
    EuropeanPenetration = 349,
    BritishExpansion = 350,
    BritishColonialAdministration = 351,
    ColonialEducation = 352,
    PeasantMovements = 353,
    TribalMovements = 354,
    IndianNationalism = 355,
    SwadeshiMovement = 356,
    GandhianMovement = 357,
    ColonialConstitutionalDevelopment = 358,
    RevolutionaryMovement = 359,
    Partition = 360,
    PostIndependenceSocialChange = 361,
    PostIndependenceEconomicDevelopment = 362,
    Imperialism = 363,
    Revolution = 364,
    ColdWar = 365,
    Underdevelopment = 366,
    EuropeanIntegration = 367,
    SovietDisintegration = 368,
    Constitutionalism = 369,
    FundamentalRights = 370,
    DirectivePrinciples = 371,
    FundamentalDuties = 372,
    President = 373,
    Governor = 374,
    LegislativePrivileges = 375,
    CivilServicesLaw = 376,
    PublicServiceCommissions = 377,
    ElectionCommission = 378,
    EmergencyProvisions = 379,
    NaturalJustice = 380,
    DelegatedLegislation = 381,
    Ombudsman = 382,
    InternationalLaw = 383,
    MunicipalLaw = 384,
    StateRecognition = 385,
    StateSuccession = 386,
    LawOfTheSea = 387,
    Nationality = 388,
    HumanRights = 389,
    StateJurisdiction = 390,
    Extradition = 391,
    Asylum = 392,
    TreatyLaw = 393,
    UnitedNations = 394,
    InternationalDisputeSettlement = 395,
    UseOfForce = 396,
    InternationalHumanitarianLaw = 397,
    NuclearLaw = 398,
    InternationalTerrorism = 399,
    InternationalCriminalLaw = 400,
    InternationalEconomicLaw = 401,
    InternationalEnvironmentalLaw = 402,
    CriminalLiability = 403,
    Punishment = 404,
    CriminalAttempt = 405,
    GeneralExceptions = 406,
    JointLiability = 407,
    Abetment = 408,
    CriminalConspiracy = 409,
    OffencesAgainstTheState = 410,
    PublicOrderOffences = 411,
    OffencesAgainstThePerson = 412,
    PropertyOffences = 413,
    OffencesAgainstWomen = 414,
    Defamation = 415,
    AntiCorruptionLaw = 416,
    CivilRightsLaw = 417,
    PleaBargaining = 418,
    TortLaw = 419,
    StrictLiability = 420,
    VicariousLiability = 421,
    Negligence = 422,
    Nuisance = 423,
    ConsumerProtection = 424,
    ContractLaw = 425,
    Indemnity = 426,
    Guarantee = 427,
    InsuranceLaw = 428,
    Agency = 429,
    SaleOfGoods = 430,
    Partnership = 431,
    NegotiableInstruments = 432,
    Arbitration = 433,
    StandardFormContracts = 434,
    PublicInterestLitigation = 435,
    CyberLaw = 436,
    CompetitionLaw = 437,
    AlternativeDisputeResolution = 438,
    TrialByMedia = 439,
    LinearAlgebra = 440,
    Calculus = 441,
    AnalyticGeometry = 442,
    OrdinaryDifferentialEquations = 443,
    Dynamics = 444,
    Statics = 445,
    VectorAnalysis = 446,
    AbstractAlgebra = 447,
    RealAnalysis = 448,
    ComplexAnalysis = 449,
    LinearProgramming = 450,
    PartialDifferentialEquations = 451,
    NumericalAnalysis = 452,
    ComputerProgramming = 453,
    AnalyticalMechanics = 454,
    FluidDynamics = 455,
    HumanAnatomy = 456,
    HumanPhysiology = 457,
    Biochemistry = 458,
    Pathology = 459,
    Microbiology = 460,
    Pharmacology = 461,
    ForensicMedicine = 462,
    Toxicology = 463,
    GeneralMedicine = 464,
    Paediatrics = 465,
    Dermatology = 466,
    GeneralSurgery = 467,
    Obstetrics = 468,
    Gynaecology = 469,
    FamilyPlanning = 470,
    Epidemiology = 471,
    Nutrition = 472,
    PublicHealthProgrammes = 473,
    HealthAdministration = 474,
    WasteManagement = 475,
    GreekPhilosophy = 476,
    Rationalism = 477,
    Empiricism = 478,
    TranscendentalIdealism = 479,
    AbsoluteIdealism = 480,
    AnalyticPhilosophy = 481,
    LogicalPositivism = 482,
    LaterWittgenstein = 483,
    Phenomenology = 484,
    Existentialism = 485,
    EpistemologicalHolism = 486,
    DescriptiveMetaphysics = 487,
    Materialism = 488,
    JainPhilosophy = 489,
    BuddhistPhilosophy = 490,
    NyayaVaisheshika = 491,
    Samkhya = 492,
    Yoga = 493,
    Mimamsa = 494,
    Vedanta = 495,
    IntegralYoga = 496,
    Equality = 497,
    Justice = 498,
    Liberty = 499,
    Sovereignty = 500,
    Duties = 501,
    FormsOfGovernment = 502,
    Humanism = 503,
    Multiculturalism = 504,
    Crime = 505,
    Development = 506,
    GenderJustice = 507,
    CasteJustice = 508,
    ConceptOfGod = 509,
    ExistenceOfGod = 510,
    ProblemOfEvil = 511,
    Soul = 512,
    Faith = 513,
    ReligiousExperience = 514,
    Atheism = 515,
    Religion = 516,
    ReligiousPluralism = 517,
    ReligiousLanguage = 518,
    PoliticalTheory = 519,
    TheoryOfTheState = 520,
    Democracy = 521,
    Power = 522,
    IndianPoliticalThought = 523,
    WesternPoliticalThought = 524,
    ConstitutionMaking = 525,
    UnionGovernment = 526,
    StateGovernment = 527,
    GrassrootsDemocracy = 528,
    StatutoryCommissions = 529,
    IdentityPolitics = 530,
    SocialMovements = 531,
    ComparativePolitics = 532,
    ComparativeState = 533,
    PoliticalRepresentation = 534,
    InternationalRelationsTheory = 535,
    InternationalSecurity = 536,
    InternationalOrder = 537,
    InternationalEconomicSystem = 538,
    GlobalIssues = 539,
    IndianForeignPolicy = 540,
    NonAlignment = 541,
    SouthAsianRelations = 542,
    GlobalSouth = 543,
    GlobalPowers = 544,
    IndiaAtTheUnitedNations = 545,
    NuclearPolicy = 546,
    ContemporaryForeignPolicy = 547,
    PublicAdministrationTheory = 548,
    AdministrativeThought = 549,
    AdministrativeBehaviour = 550,
    Organisations = 551,
    AdministrativeAccountability = 552,
    AdministrativeLaw = 553,
    ComparativePublicAdministration = 554,
    DevelopmentAdministration = 555,
    PersonnelAdministration = 556,
    AdministrativeImprovement = 557,
    FinancialAdministration = 558,
    EvolutionOfIndianAdministration = 559,
    ConstitutionalAdministration = 560,
    PublicSectorUndertakings = 561,
    UnionAdministration = 562,
    StateAdministration = 563,
    DistrictAdministration = 564,
    PublicFinancialManagement = 565,
    AdministrativeReforms = 566,
    UrbanLocalGovernment = 567,
    PublicOrderAdministration = 568,
    PublicServiceEthics = 569,
    RegulatoryCommissions = 570,
    HumanRightsAdministration = 571,
    CoalitionAdministration = 572,
    DisasterAdministration = 573,
    SociologicalDiscipline = 574,
    SociologyOfScience = 575,
    SociologicalResearchMethods = 576,
    SociologicalThinkers = 577,
    SocialStratification = 578,
    SocialMobility = 579,
    SociologyOfWork = 580,
    PoliticalSociology = 581,
    SociologyOfReligion = 582,
    SocialChange = 583,
    IndianSociology = 584,
    ColonialSocialChange = 585,
    RuralSocialStructure = 586,
    SocialClass = 587,
    KinshipInIndia = 588,
    ReligiousCommunities = 589,
    DevelopmentPlanning = 590,
    AgrarianTransformation = 591,
    Industrialisation = 592,
    PoliticsInIndianSociety = 593,
    WomenMovements = 594,
    DalitMovements = 595,
    EnvironmentalMovements = 596,
    IdentityMovements = 597,
    PopulationDynamics = 598,
    Displacement = 599,
    ViolenceAgainstWomen = 600,
    CasteConflict = 601,
    EthnicConflict = 602,
    EducationalInequality = 603,
    Philosophy = 604,
    HumanNature = 605,
    Governance = 606,
    Economy = 607,
    Science = 608,
    Technology = 609,
    Environment = 610,
    Peace = 611,
    Media = 612,
});

numeric_enum!(Subtag {
    Nationalism = 0,
    HumanGeography = 1,
    EconomicGeography = 2,
    Biogeography = 3,
    NaturalResources = 4,
    Constitution = 5,
    Judiciary = 6,
    Federalism = 7,
    LocalSelfGovernment = 8,
    PartySystem = 9,
    PublicPolicy = 10,
    Rights = 11,
    CivilServices = 12,
    EconomicDevelopment = 13,
    EconomicGrowth = 14,
    EconomicPlanning = 15,
    Microeconomics = 16,
    FiscalPolicy = 17,
    Taxation = 18,
    MonetaryPolicy = 19,
    Banking = 20,
    FinancialMarkets = 21,
    BalanceOfPayments = 22,
    InternationalTrade = 23,
    Employment = 24,
    FinancialManagement = 25,
    Poverty = 26,
    TribalCommunities = 27,
    Education = 28,
    Health = 29,
    EnvironmentalLaw = 30,
    HumanEvolution = 31,
    EthicalDilemmas = 32,
    PoliticalIdeologies = 33,
    SocialReformMovements = 34,
    CommunicationSkills = 35,
    DecisionMaking = 36,
    ProblemSolving = 37,
    GeneralMentalAbility = 38,
    PostIndependenceConsolidation = 39,
    DemocraticRevolutions = 40,
    Enlightenment = 41,
    IndustrialRevolution = 42,
    WorldWars = 43,
    Decolonisation = 44,
    Caste = 45,
    Family = 46,
    Urbanisation = 47,
    Globalisation = 48,
    Communalism = 49,
    Regionalism = 50,
    Secularism = 51,
    Geomorphology = 52,
    Climatology = 53,
    Oceanography = 54,
    ConstitutionalAmendment = 55,
    SeparationOfPowers = 56,
    JudicialReview = 57,
    IndustrialPolicy = 58,
    IntellectualPropertyRights = 59,
    Ethics = 60,
    Attitude = 61,
    Persuasion = 62,
    Aptitude = 63,
    Integrity = 64,
    Impartiality = 65,
    DedicationToPublicService = 66,
    Compassion = 67,
    ConflictOfInterest = 68,
    EthicalGovernance = 69,
    Probity = 70,
    RightToInformation = 71,
    CodeOfEthics = 72,
    CodeOfConduct = 73,
    ServiceDelivery = 74,
    PublicFunds = 75,
    Corruption = 76,
    AnthropologicalScope = 77,
    SocioculturalAnthropology = 78,
    BiologicalAnthropology = 79,
    ArchaeologicalAnthropology = 80,
    LinguisticAnthropology = 81,
    Primatology = 82,
    Palaeoanthropology = 83,
    MolecularAnthropology = 84,
    PrehistoricArchaeology = 85,
    Culture = 86,
    Society = 87,
    Marriage = 88,
    Kinship = 89,
    EconomicAnthropology = 90,
    PoliticalAnthropology = 91,
    AnthropologyOfReligion = 92,
    AnthropologicalTheory = 93,
    Language = 94,
    AnthropologicalResearchMethods = 95,
    HumanGenetics = 96,
    MendelianGenetics = 97,
    PopulationGenetics = 98,
    ChromosomalDisorders = 99,
    Race = 100,
    EcologicalAnthropology = 101,
    EpidemiologicalAnthropology = 102,
    HumanGrowth = 103,
    Fertility = 104,
    DemographicTheory = 105,
    AppliedAnthropology = 106,
    IndianPrehistory = 107,
    IndianPalaeoanthropology = 108,
    Ethnoarchaeology = 109,
    IndianDemography = 110,
    IndianSocialSystem = 111,
    SacredComplex = 112,
    ReligiousChange = 113,
    IndianAnthropology = 114,
    IndianVillage = 115,
    Minorities = 116,
    SocioculturalChange = 117,
    TribalDemography = 118,
    TribalProblems = 119,
    TribalDisplacement = 120,
    ConstitutionalSafeguards = 121,
    TribalChange = 122,
    Ethnicity = 123,
    TribalReligion = 124,
    TribeStateRelations = 125,
    TribalAdministration = 126,
    TribalDevelopment = 127,
    RuralDevelopment = 128,
    FinancialAccounting = 129,
    CostAccounting = 130,
    Auditing = 131,
    FinancialInstitutions = 132,
    OrganisationTheory = 133,
    OrganisationalBehaviour = 134,
    HumanResourceManagement = 135,
    IndustrialRelations = 136,
    WelfareEconomics = 137,
    Macroeconomics = 138,
    MonetaryEconomics = 139,
    PublicFinance = 140,
    HumanDevelopment = 141,
    EnvironmentalEconomics = 142,
    ColonialEconomy = 143,
    AgriculturalDevelopment = 144,
    IndustrialDevelopment = 145,
    NationalIncome = 146,
    AgriculturalPolicy = 147,
    TradePolicy = 148,
    ExchangeRatePolicy = 149,
    EnvironmentalGeography = 150,
    PopulationGeography = 151,
    SettlementGeography = 152,
    RegionalPlanning = 153,
    GeographicalModels = 154,
    PhysicalSettingOfIndia = 155,
    AgriculturalGeography = 156,
    IndustrialGeography = 157,
    TransportGeography = 158,
    CommunicationGeography = 159,
    TradeGeography = 160,
    CulturalGeographyOfIndia = 161,
    SettlementGeographyOfIndia = 162,
    RegionalDevelopment = 163,
    PoliticalGeography = 164,
    ContemporaryGeographicalIssues = 165,
    MapInterpretation = 166,
    HistoricalMapWork = 167,
    HistoricalSources = 168,
    Prehistory = 169,
    IndusValleyCivilisation = 170,
    MegalithicCultures = 171,
    VedicPeriod = 172,
    Mahajanapadas = 173,
    MauryanEmpire = 174,
    PostMauryanIndia = 175,
    EarlyPeninsularIndia = 176,
    GuptaAge = 177,
    EarlyIndianCulture = 178,
    EarlyMedievalIndia = 179,
    MedievalCulturalTraditions = 180,
    DelhiSultanate = 181,
    KhaljiDynasty = 182,
    TughlaqDynasty = 183,
    SultanateSociety = 184,
    VijayanagaraEmpire = 185,
    MughalFoundation = 186,
    MughalConsolidation = 187,
    SeventeenthCenturyMughalEmpire = 188,
    MughalEconomy = 189,
    MughalCulture = 190,
    EighteenthCenturyIndia = 191,
    EuropeanPenetration = 192,
    BritishExpansion = 193,
    ColonialEducation = 194,
    PeasantMovements = 195,
    TribalMovements = 196,
    IndianNationalism = 197,
    SwadeshiMovement = 198,
    GandhianMovement = 199,
    ColonialConstitutionalDevelopment = 200,
    RevolutionaryMovement = 201,
    Partition = 202,
    PostIndependenceSocialChange = 203,
    PostIndependenceEconomicDevelopment = 204,
    Imperialism = 205,
    Revolution = 206,
    ColdWar = 207,
    Underdevelopment = 208,
    EuropeanIntegration = 209,
    SovietDisintegration = 210,
    Constitutionalism = 211,
    FundamentalRights = 212,
    DirectivePrinciples = 213,
    FundamentalDuties = 214,
    President = 215,
    Governor = 216,
    LegislativePrivileges = 217,
    CivilServicesLaw = 218,
    PublicServiceCommissions = 219,
    ElectionCommission = 220,
    EmergencyProvisions = 221,
    NaturalJustice = 222,
    DelegatedLegislation = 223,
    Ombudsman = 224,
    InternationalLaw = 225,
    MunicipalLaw = 226,
    StateRecognition = 227,
    StateSuccession = 228,
    LawOfTheSea = 229,
    Nationality = 230,
    HumanRights = 231,
    StateJurisdiction = 232,
    Extradition = 233,
    Asylum = 234,
    TreatyLaw = 235,
    UnitedNations = 236,
    InternationalDisputeSettlement = 237,
    UseOfForce = 238,
    InternationalHumanitarianLaw = 239,
    NuclearLaw = 240,
    InternationalTerrorism = 241,
    InternationalCriminalLaw = 242,
    InternationalEconomicLaw = 243,
    InternationalEnvironmentalLaw = 244,
    CriminalLiability = 245,
    Punishment = 246,
    CriminalAttempt = 247,
    GeneralExceptions = 248,
    JointLiability = 249,
    Abetment = 250,
    CriminalConspiracy = 251,
    OffencesAgainstTheState = 252,
    PublicOrderOffences = 253,
    OffencesAgainstThePerson = 254,
    PropertyOffences = 255,
    OffencesAgainstWomen = 256,
    Defamation = 257,
    AntiCorruptionLaw = 258,
    CivilRightsLaw = 259,
    PleaBargaining = 260,
    TortLaw = 261,
    StrictLiability = 262,
    VicariousLiability = 263,
    Negligence = 264,
    Nuisance = 265,
    ConsumerProtection = 266,
    ContractLaw = 267,
    Indemnity = 268,
    Guarantee = 269,
    InsuranceLaw = 270,
    Agency = 271,
    SaleOfGoods = 272,
    Partnership = 273,
    NegotiableInstruments = 274,
    Arbitration = 275,
    StandardFormContracts = 276,
    PublicInterestLitigation = 277,
    CyberLaw = 278,
    CompetitionLaw = 279,
    AlternativeDisputeResolution = 280,
    TrialByMedia = 281,
    LinearAlgebra = 282,
    Calculus = 283,
    AnalyticGeometry = 284,
    OrdinaryDifferentialEquations = 285,
    Dynamics = 286,
    Statics = 287,
    VectorAnalysis = 288,
    AbstractAlgebra = 289,
    RealAnalysis = 290,
    ComplexAnalysis = 291,
    LinearProgramming = 292,
    PartialDifferentialEquations = 293,
    NumericalAnalysis = 294,
    ComputerProgramming = 295,
    AnalyticalMechanics = 296,
    FluidDynamics = 297,
    HumanAnatomy = 298,
    HumanPhysiology = 299,
    Biochemistry = 300,
    Pathology = 301,
    Microbiology = 302,
    Pharmacology = 303,
    ForensicMedicine = 304,
    Toxicology = 305,
    GeneralMedicine = 306,
    Paediatrics = 307,
    Dermatology = 308,
    GeneralSurgery = 309,
    Obstetrics = 310,
    Gynaecology = 311,
    FamilyPlanning = 312,
    Epidemiology = 313,
    Nutrition = 314,
    PublicHealthProgrammes = 315,
    HealthAdministration = 316,
    WasteManagement = 317,
    GreekPhilosophy = 318,
    Rationalism = 319,
    Empiricism = 320,
    TranscendentalIdealism = 321,
    AbsoluteIdealism = 322,
    AnalyticPhilosophy = 323,
    LogicalPositivism = 324,
    LaterWittgenstein = 325,
    Phenomenology = 326,
    Existentialism = 327,
    EpistemologicalHolism = 328,
    DescriptiveMetaphysics = 329,
    Materialism = 330,
    JainPhilosophy = 331,
    BuddhistPhilosophy = 332,
    NyayaVaisheshika = 333,
    Samkhya = 334,
    Yoga = 335,
    Mimamsa = 336,
    Vedanta = 337,
    IntegralYoga = 338,
    Equality = 339,
    Justice = 340,
    Liberty = 341,
    Sovereignty = 342,
    Duties = 343,
    FormsOfGovernment = 344,
    Humanism = 345,
    Multiculturalism = 346,
    Crime = 347,
    Development = 348,
    GenderJustice = 349,
    CasteJustice = 350,
    ConceptOfGod = 351,
    ExistenceOfGod = 352,
    ProblemOfEvil = 353,
    Soul = 354,
    Faith = 355,
    ReligiousExperience = 356,
    Atheism = 357,
    Religion = 358,
    ReligiousPluralism = 359,
    ReligiousLanguage = 360,
    PoliticalTheory = 361,
    TheoryOfTheState = 362,
    Democracy = 363,
    Power = 364,
    IndianPoliticalThought = 365,
    WesternPoliticalThought = 366,
    ConstitutionMaking = 367,
    UnionGovernment = 368,
    StateGovernment = 369,
    GrassrootsDemocracy = 370,
    StatutoryCommissions = 371,
    IdentityPolitics = 372,
    SocialMovements = 373,
    ComparativePolitics = 374,
    ComparativeState = 375,
    PoliticalRepresentation = 376,
    InternationalRelationsTheory = 377,
    InternationalSecurity = 378,
    InternationalOrder = 379,
    InternationalEconomicSystem = 380,
    GlobalIssues = 381,
    IndianForeignPolicy = 382,
    NonAlignment = 383,
    SouthAsianRelations = 384,
    GlobalSouth = 385,
    GlobalPowers = 386,
    IndiaAtTheUnitedNations = 387,
    NuclearPolicy = 388,
    ContemporaryForeignPolicy = 389,
    PublicAdministrationTheory = 390,
    AdministrativeThought = 391,
    AdministrativeBehaviour = 392,
    Organisations = 393,
    AdministrativeAccountability = 394,
    AdministrativeLaw = 395,
    ComparativePublicAdministration = 396,
    DevelopmentAdministration = 397,
    PersonnelAdministration = 398,
    AdministrativeImprovement = 399,
    FinancialAdministration = 400,
    EvolutionOfIndianAdministration = 401,
    ConstitutionalAdministration = 402,
    PublicSectorUndertakings = 403,
    UnionAdministration = 404,
    StateAdministration = 405,
    DistrictAdministration = 406,
    PublicFinancialManagement = 407,
    AdministrativeReforms = 408,
    UrbanLocalGovernment = 409,
    PublicOrderAdministration = 410,
    PublicServiceEthics = 411,
    RegulatoryCommissions = 412,
    HumanRightsAdministration = 413,
    CoalitionAdministration = 414,
    DisasterAdministration = 415,
    SociologicalDiscipline = 416,
    SociologyOfScience = 417,
    SociologicalResearchMethods = 418,
    SociologicalThinkers = 419,
    SocialStratification = 420,
    SocialMobility = 421,
    SociologyOfWork = 422,
    PoliticalSociology = 423,
    SociologyOfReligion = 424,
    SocialChange = 425,
    IndianSociology = 426,
    ColonialSocialChange = 427,
    RuralSocialStructure = 428,
    SocialClass = 429,
    KinshipInIndia = 430,
    ReligiousCommunities = 431,
    DevelopmentPlanning = 432,
    AgrarianTransformation = 433,
    Industrialisation = 434,
    PoliticsInIndianSociety = 435,
    WomenMovements = 436,
    DalitMovements = 437,
    EnvironmentalMovements = 438,
    IdentityMovements = 439,
    PopulationDynamics = 440,
    Displacement = 441,
    ViolenceAgainstWomen = 442,
    CasteConflict = 443,
    EthnicConflict = 444,
    EducationalInequality = 445,
    Governance = 446,
    Technology = 447,
    Peace = 448,
    Media = 449,
    Truth = 450,
    Freedom = 451,
    Responsibility = 452,
    Leadership = 453,
    Knowledge = 454,
    Nature = 455,
    Sustainability = 456,
    War = 457,
    Identity = 458,
    Work = 459,
    Community = 460,
    Dignity = 461,
    Change = 462,
    Time = 463,
    RiskTaking = 464,
    HumanPotential = 465,
    Resilience = 466,
    Happiness = 467,
    Creativity = 468,
    Spirituality = 469,
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionTaxonomy {
    pub main_tag: MainTag,
    #[serde(default)]
    pub subtags: Vec<Subtag>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaxonomyLabels {
    pub version: u16,
    #[serde(default)]
    pub legacy_main_tag_aliases: Vec<LegacyMainTagAlias>,
    pub main_tags: Vec<MainTagLabel>,
    pub subtags: Vec<SubtagLabel>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyMainTagAlias {
    pub label: String,
    #[serde(default)]
    pub main_tags: Vec<String>,
    #[serde(default)]
    pub sections: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct MainTagLabel {
    pub id: u16,
    pub label: String,
    pub description: String,
    pub subtags: Vec<u16>,
}

#[derive(Debug, Deserialize)]
pub struct SubtagLabel {
    pub id: u16,
    pub label: String,
    pub description: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ResolvedTaxonomy<'a> {
    pub main_tag: &'a str,
    pub subtags: Vec<&'a str>,
}

static LABELS: OnceLock<TaxonomyLabels> = OnceLock::new();

pub fn labels() -> &'static TaxonomyLabels {
    LABELS.get_or_init(|| {
        let labels: TaxonomyLabels =
            serde_json::from_str(LABELS_JSON).expect("bundled taxonomy labels must be valid JSON");
        assert_eq!(
            labels.version, TAXONOMY_VERSION,
            "unsupported bundled taxonomy version"
        );
        labels
    })
}

fn normalized_label(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .replace(" and ", " & ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Resolve a retired broad tag to its compatibility scope. The replacement
/// labels remain atomic; optional-subject placeholders map to paper sections
/// because paper identity is metadata rather than a current main tag.
pub fn legacy_main_tag_alias(label: &str) -> Option<&'static LegacyMainTagAlias> {
    let normalized = normalized_label(label);
    labels()
        .legacy_main_tag_aliases
        .iter()
        .find(|alias| normalized_label(&alias.label) == normalized)
}

impl QuestionTaxonomy {
    /// Reconstruct typed taxonomy from the human-readable labels stored in
    /// SQLite's taxonomy projection when returning questions over IPC.
    pub fn from_labels(main_label: &str, subtag_labels: &[String]) -> Result<Self, String> {
        let labels = labels();
        let candidates = labels
            .main_tags
            .iter()
            .filter(|label| label.label == main_label)
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return Err(format!("Unknown taxonomy main-tag label: {main_label}"));
        }

        for main in candidates {
            let mut subtags = Vec::with_capacity(subtag_labels.len());
            let mut matches_main = true;
            for subtag_label in subtag_labels {
                let matching_id = main.subtags.iter().find_map(|id| {
                    labels
                        .subtags
                        .iter()
                        .find(|label| label.id == *id && label.label == *subtag_label)
                        .map(|_| *id)
                });
                let Some(id) = matching_id else {
                    matches_main = false;
                    break;
                };
                subtags.push(Subtag::try_from(id)?);
            }

            if matches_main {
                let taxonomy = Self {
                    main_tag: MainTag::try_from(main.id)?,
                    subtags,
                };
                taxonomy.resolve()?;
                return Ok(taxonomy);
            }
        }

        Err(format!(
            "Taxonomy labels do not form a valid main/subtag combination: {main_label}"
        ))
    }

    pub fn resolve(&self) -> Result<ResolvedTaxonomy<'static>, String> {
        if self.subtags.len() > MAX_SUBTAGS {
            return Err(format!(
                "Taxonomy has {} subtags; maximum is {MAX_SUBTAGS}",
                self.subtags.len()
            ));
        }

        let labels = labels();
        let main_id = usize::from(self.main_tag.id());
        let main = labels
            .main_tags
            .get(main_id)
            .filter(|label| usize::from(label.id) == main_id)
            .ok_or_else(|| format!("Missing taxonomy main-tag label for ID {main_id}"))?;

        let mut seen = HashSet::new();
        let mut resolved_subtags = Vec::with_capacity(self.subtags.len());
        for subtag in &self.subtags {
            let subtag_id = subtag.id();
            if !seen.insert(subtag_id) {
                return Err(format!("Duplicate taxonomy subtag ID: {subtag_id}"));
            }
            if !main.subtags.contains(&subtag_id) {
                return Err(format!(
                    "Taxonomy subtag ID {subtag_id} does not belong to main tag {}",
                    main.label
                ));
            }
            let label = labels
                .subtags
                .iter()
                .find(|label| label.id == subtag_id)
                .ok_or_else(|| format!("Missing taxonomy subtag label for ID {subtag_id}"))?;
            resolved_subtags.push(label.label.as_str());
        }

        Ok(ResolvedTaxonomy {
            main_tag: main.label.as_str(),
            subtags: resolved_subtags,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_registry_is_complete() {
        let labels = labels();
        assert_eq!(labels.main_tags.len(), 613);
        assert_eq!(labels.subtags.len(), 470);

        let subtag_names = labels
            .subtags
            .iter()
            .map(|tag| tag.label.as_str())
            .collect::<HashSet<_>>();
        let registered_subtags = labels
            .subtags
            .iter()
            .map(|tag| tag.id)
            .collect::<HashSet<_>>();
        let active_subtags = labels
            .main_tags
            .iter()
            .flat_map(|tag| tag.subtags.iter().copied())
            .collect::<HashSet<_>>();
        assert!(
            !subtag_names.is_empty(),
            "taxonomy must register at least one subtag"
        );
        assert_eq!(registered_subtags, active_subtags);

        for (index, main) in labels.main_tags.iter().enumerate() {
            assert_eq!(usize::from(main.id), index);
            assert!(!main.label.trim().is_empty());
            assert!(!main.description.trim().is_empty());
            let normalized = main.label.to_lowercase();
            assert!(!main.label.contains(" & "));
            assert!(!main.label.contains('/'));
            assert!(!normalized.contains(" and "));
            assert!(!main.label.contains(','));
            for subtag in &main.subtags {
                assert!(labels.subtags.iter().any(|label| label.id == *subtag));
            }
        }
        for subtag in &labels.subtags {
            assert!(!subtag.label.trim().is_empty());
            assert!(!subtag.description.trim().is_empty());
        }

        let main_tag_names = labels
            .main_tags
            .iter()
            .map(|tag| tag.label.as_str())
            .collect::<HashSet<_>>();
        let mut aliases = HashSet::new();
        for alias in &labels.legacy_main_tag_aliases {
            assert!(!alias.label.trim().is_empty());
            assert!(aliases.insert(normalized_label(&alias.label)));
            assert!(!alias.main_tags.is_empty() || !alias.sections.is_empty());
            assert!(alias
                .main_tags
                .iter()
                .all(|main_tag| main_tag_names.contains(main_tag.as_str())));
            assert!(alias
                .sections
                .iter()
                .all(|section| !section.trim().is_empty()));
        }
    }

    #[test]
    fn numeric_taxonomy_round_trips_and_validates_membership() {
        let taxonomy: QuestionTaxonomy =
            serde_json::from_str(r#"{"mainTag":7,"subtags":[34]}"#).unwrap();
        assert_eq!(taxonomy.main_tag.id(), 7);
        assert_eq!(taxonomy.subtags[0].id(), 34);
        assert_eq!(
            taxonomy.resolve().unwrap(),
            ResolvedTaxonomy {
                main_tag: "Nationalism",
                subtags: vec!["Social Reform Movements"],
            }
        );
        assert_eq!(
            serde_json::to_string(&taxonomy).unwrap(),
            r#"{"mainTag":7,"subtags":[34]}"#
        );

        let invalid = QuestionTaxonomy {
            main_tag: MainTag::try_from(0).unwrap(),
            subtags: vec![Subtag::try_from(34).unwrap()],
        };
        assert!(invalid.resolve().is_err());
    }

    #[test]
    fn expanded_taxonomy_resolves_essay_mathematics_and_optional_topics() {
        let cases = [
            (r#"{"mainTag":440,"subtags":[]}"#, "Linear Algebra"),
            (r#"{"mainTag":456,"subtags":[]}"#, "Human Anatomy"),
            (r#"{"mainTag":476,"subtags":[]}"#, "Greek Philosophy"),
            (r#"{"mainTag":355,"subtags":[]}"#, "Indian Nationalism"),
            (r#"{"mainTag":604,"subtags":[]}"#, "Philosophy"),
        ];

        for (json, expected_main) in cases {
            let taxonomy: QuestionTaxonomy = serde_json::from_str(json).unwrap();
            assert_eq!(taxonomy.resolve().unwrap().main_tag, expected_main);
        }
    }

    #[test]
    fn label_readback_resolves_repeated_subtag_labels_in_their_main_family() {
        let mathematics = QuestionTaxonomy::from_labels("Linear Algebra", &[]).unwrap();
        assert_eq!(mathematics.main_tag.id(), 440);
        assert!(mathematics.subtags.is_empty());

        let essay = QuestionTaxonomy::from_labels("Philosophy", &["Truth".to_string()]).unwrap();
        assert_eq!(essay.main_tag.id(), 604);
        assert_eq!(essay.subtags[0].id(), 450);
    }

    #[test]
    fn retired_broad_tags_resolve_to_atomic_topics_or_paper_metadata() {
        let polity = legacy_main_tag_alias("polity and constitution").unwrap();
        assert!(polity.main_tags.iter().any(|tag| tag == "Constitution"));
        assert!(polity.main_tags.iter().any(|tag| tag == "Parliament"));

        let mathematics = legacy_main_tag_alias("Quantitative & Mathematical Methods").unwrap();
        assert!(mathematics.main_tags.is_empty());
        assert_eq!(
            mathematics.sections,
            ["mains-maths1".to_string(), "mains-maths2".to_string()]
        );
    }
}
