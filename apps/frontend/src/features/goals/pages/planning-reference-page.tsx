import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger,
  Alert,
  AlertDescription,
  AlertTitle,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Page,
  PageContent,
  PageHeader,
} from "@wealthfolio/ui";
import { useTranslation } from "react-i18next";
import { Link, useNavigate } from "react-router-dom";

export default function PlanningReferencePage() {
  const { t } = useTranslation();
  const navigate = useNavigate();

  return (
    <Page>
      <PageHeader
        heading={t("goals:planning_reference.title")}
        text={t("goals:planning_reference.subtitle")}
        onBack={() => navigate("/goals")}
      />
      <PageContent>
        <div className="mx-auto max-w-4xl space-y-6">
          <Alert>
            <AlertTitle>{t("goals:planning_reference.disclaimer_title")}</AlertTitle>
            <AlertDescription>{t("goals:planning_reference.disclaimer")}</AlertDescription>
          </Alert>

          <section aria-labelledby="borrowing-reference">
            <Card>
              <CardHeader>
                <CardTitle id="borrowing-reference">
                  {t("goals:planning_reference.borrowing.title")}
                </CardTitle>
                <CardDescription>
                  {t("goals:planning_reference.borrowing.description")}
                </CardDescription>
              </CardHeader>
              <CardContent>
                <Accordion type="single" collapsible>
                  <AccordionItem value="amortization">
                    <AccordionTrigger>
                      {t("goals:planning_reference.borrowing.amortization.title")}
                    </AccordionTrigger>
                    <AccordionContent>
                      {t("goals:planning_reference.borrowing.amortization.text")}
                    </AccordionContent>
                  </AccordionItem>
                  <AccordionItem value="housing-costs">
                    <AccordionTrigger>
                      {t("goals:planning_reference.borrowing.housing_costs.title")}
                    </AccordionTrigger>
                    <AccordionContent>
                      {t("goals:planning_reference.borrowing.housing_costs.text")}
                    </AccordionContent>
                  </AccordionItem>
                  <AccordionItem value="ratios">
                    <AccordionTrigger>
                      {t("goals:planning_reference.borrowing.ratios.title")}
                    </AccordionTrigger>
                    <AccordionContent>
                      {t("goals:planning_reference.borrowing.ratios.text")}
                    </AccordionContent>
                  </AccordionItem>
                  <AccordionItem value="stress">
                    <AccordionTrigger>
                      {t("goals:planning_reference.borrowing.stress.title")}
                    </AccordionTrigger>
                    <AccordionContent>
                      {t("goals:planning_reference.borrowing.stress.text")}
                    </AccordionContent>
                  </AccordionItem>
                </Accordion>
              </CardContent>
            </Card>
          </section>

          <section aria-labelledby="retirement-reference">
            <Card>
              <CardHeader>
                <CardTitle id="retirement-reference">
                  {t("goals:planning_reference.retirement.title")}
                </CardTitle>
                <CardDescription>
                  {t("goals:planning_reference.retirement.description")}
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-5">
                <Accordion type="single" collapsible>
                  <AccordionItem value="purchasing-power">
                    <AccordionTrigger>
                      {t("goals:planning_reference.retirement.purchasing_power.title")}
                    </AccordionTrigger>
                    <AccordionContent>
                      {t("goals:planning_reference.retirement.purchasing_power.text")}
                    </AccordionContent>
                  </AccordionItem>
                  <AccordionItem value="uncertainty">
                    <AccordionTrigger>
                      {t("goals:planning_reference.retirement.uncertainty.title")}
                    </AccordionTrigger>
                    <AccordionContent>
                      {t("goals:planning_reference.retirement.uncertainty.text")}
                    </AccordionContent>
                  </AccordionItem>
                  <AccordionItem value="ranges">
                    <AccordionTrigger>
                      {t("goals:planning_reference.retirement.ranges.title")}
                    </AccordionTrigger>
                    <AccordionContent>
                      {t("goals:planning_reference.retirement.ranges.text")}
                    </AccordionContent>
                  </AccordionItem>
                </Accordion>
                <Button asChild variant="outline">
                  <Link to="/goals/retirement-guide">
                    {t("goals:planning_reference.retirement.cta")}
                  </Link>
                </Button>
              </CardContent>
            </Card>
          </section>

          <section aria-labelledby="us-investment-order">
            <Card>
              <CardHeader>
                <CardTitle id="us-investment-order">
                  {t("goals:planning_reference.investment.title")}
                </CardTitle>
                <CardDescription>
                  {t("goals:planning_reference.investment.description")}
                </CardDescription>
              </CardHeader>
              <CardContent>
                <ol className="text-muted-foreground list-decimal space-y-3 pl-5 text-sm leading-relaxed">
                  <li>{t("goals:planning_reference.investment.step_one")}</li>
                  <li>{t("goals:planning_reference.investment.step_two")}</li>
                  <li>{t("goals:planning_reference.investment.step_three")}</li>
                  <li>{t("goals:planning_reference.investment.step_four")}</li>
                  <li>{t("goals:planning_reference.investment.step_five")}</li>
                  <li>{t("goals:planning_reference.investment.step_six")}</li>
                  <li>{t("goals:planning_reference.investment.step_seven")}</li>
                </ol>
              </CardContent>
            </Card>
          </section>
        </div>
      </PageContent>
    </Page>
  );
}
